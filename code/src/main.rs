use std::num::NonZeroU32;

use anyhow::Result;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{InputPin, InterruptType, Level, OutputPin, Pin, PinDriver, Pull};
use esp_idf_svc::hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::hal::prelude::{FromValueType, Peripherals};
use esp_idf_svc::hal::task::notification::Notification;
use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use veml7700::Veml7700;

use crate::error::Error;

mod error;

const LED_POWER_STEPS: usize = 100;

// max. reaction delay when LED Power Phase is in Off or ON state
const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step/max-reaction delay when LED Power Phase is in PowerDown or PowerUp state
const LED_DIMM_DOWN_STEP_DELAY_MS: u32 = 200;

const LED_DIMM_UP_STEP_DELAY_MS: u32 = 65;


// in range [0..LED_POWER_STEPS]
// translates to power level in range [0..100] via cubic curve y=x³
fn led_power_level(step: usize) -> usize {
    (step as f32 / 100_f32).powi(3).round() as usize
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Phase {
    Off,
    PowerDown,
    PowerUp,
    On,
}

#[derive(Clone, Eq, PartialEq, Debug)]
struct State {
    // ambient light level history buffer (last 10 values)
    pub ambient_light_sensor_lux_buffer: ConstGenericRingBuffer<u32, 10>,
    pub phase: Phase,
    pub led_power_step: usize,
}

impl State {
    pub fn new() -> Self {
        State {
            ambient_light_sensor_lux_buffer: ConstGenericRingBuffer::new(),
            phase: Phase::Off,
            led_power_step: 0,
        }
    }
    pub fn is_dark_enough_for_operation(&self) -> bool {
        

        // TODO determine active/passive state based on ambient light level history
        //  => take median values
        todo!()
    }

    pub fn duty_step_delay_ms(&self) -> u32 {
        match self.phase {
            Phase::Off | Phase::On => ON_OFF_REACTION_STEP_DELAY_MS,
            Phase::PowerDown => LED_DIMM_DOWN_STEP_DELAY_MS,
            Phase::PowerUp => LED_DIMM_UP_STEP_DELAY_MS
        }
    }

    pub fn calc_dimm_progress(&mut self) {
        match self.phase {
            Phase::Off => debug_assert_eq!(self.led_power_step, 0),
            Phase::PowerDown => {
                if self.led_power_step > 0 {
                    self.led_power_step -= 1;
                    if self.led_power_step == 0 {
                        self.phase = Phase::Off;
                    }
                }
            }
            Phase::PowerUp => {
                if self.led_power_step < LED_POWER_STEPS {
                    self.led_power_step += 1;
                    if self.led_power_step == LED_POWER_STEPS {
                        self.phase = Phase::On;
                    }
                }
            }
            Phase::On => debug_assert_eq!(self.led_power_step, LED_POWER_STEPS)
        }
    }
}

struct PresenceSensor<P1: Pin> {
    pub gpio_pin: PinDriver<'static, P1, gpio::Input>,
    pub notification: Notification,
}

struct Devices<P1: Pin>
{
    pub presence_sensor: PresenceSensor<P1>,
    pub ambient_light_sensor: Veml7700<I2cDriver<'static>>,
}

impl<P1: Pin> Devices<P1>
{
    pub fn read_sensors(&mut self, state: &mut State) -> Result<()> {
        if state.phase == Phase::Off {
            self.measure_ambient_light_level(state)?;
        }

        if let Some(_) = self.presence_sensor.notification.wait(esp_idf_svc::hal::delay::NON_BLOCK) {
            if state.is_dark_enough_for_operation() {
                self.on_presence_sensor_event(state);
                // re-enable interrupt after we have been triggering (disables the interrupt automatically)
                self.presence_sensor.gpio_pin.enable_interrupt()?;
            }
        }
        Ok(())
    }

    // measure ambient light level - only if LED is Off
    fn measure_ambient_light_level(&mut self, state: &mut State) -> Result<()> {
        let lux: u32 = self.ambient_light_sensor.read_lux()
            .map_err(|e| Error::from(e))?.round() as u32;
        state.ambient_light_sensor_lux_buffer.enqueue(lux);
        Ok(())
    }

    fn on_presence_sensor_event(&mut self, state: &mut State) {
        match self.presence_sensor.gpio_pin.get_level() {
            Level::Low => {
                state.phase = Phase::PowerDown;
                log::debug!("Dimming down");
            }
            Level::High => {
                state.phase = Phase::PowerUp;
                log::debug!("Dimming up");
            }
        }
    }

    pub fn apply_led_power_level(&self, _bar_state: &State) {
        // TODO LED PWM interface
        todo!()
    }
}


fn init_presence_sensor_on_interrupt<P: InputPin + OutputPin>(
    gpio_pin: P
) -> Result<PresenceSensor<P>> {

    // radar presence sensor
    let mut pin_driver = PinDriver::input(gpio_pin)?;
    pin_driver.set_pull(Pull::Floating)?;
    pin_driver.set_interrupt_type(InterruptType::AnyEdge)?;

    let notification = Notification::new();
    let notifier = notification.notifier();

    // Safety: make sure the `Notification` object is not dropped while the subscription is active
    unsafe {
        pin_driver.subscribe(move || {
            notifier.notify_and_yield(NonZeroU32::new(1).unwrap());
        })?;
    }

    // initial enable of interrupt
    pin_driver.enable_interrupt()?;

    Ok(PresenceSensor {
        gpio_pin: pin_driver,
        notification,
    })
}

fn init_veml7700<I2C>(
    i2c: impl Peripheral<P=I2C> + 'static,
    sda: impl Peripheral<P=impl InputPin + OutputPin> + 'static,
    scl: impl Peripheral<P=impl InputPin + OutputPin> + 'static,
) -> Result<Veml7700<I2cDriver<'static>>>
where I2C: I2c,
{
    let config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c_driver = I2cDriver::new(i2c, sda, scl, &config)?;

    // Initialize the VEML7700 with the I2C
    let mut veml7700_device = Veml7700::new(i2c_driver);
    veml7700_device.enable().map_err(|e| Error::from(e))?;
    Ok(veml7700_device)
}

fn main() -> Result<()> {
    log::info!("on.");
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();

    let presence_sensor = init_presence_sensor_on_interrupt(peripherals.pins.gpio12)?;

    let ambient_light_sensor = init_veml7700(
        peripherals.i2c0,
        peripherals.pins.gpio10,
        peripherals.pins.gpio11,
    )?;

    let mut devices = Devices {
        presence_sensor,
        ambient_light_sensor,
    };

    let mut bar_state = State::new();

    loop {
        FreeRtos::delay_ms(bar_state.duty_step_delay_ms());
        devices.read_sensors(&mut bar_state)?;
        bar_state.calc_dimm_progress();
        devices.apply_led_power_level(&bar_state);
    }
}


// TODO find a suitable project name (working project name "led-sensor-bar").
//  Candidates:
//  - smart-night-light
// TODO implement a faster dimm-up and slower dimm-down for the LED
// TODO disable unused pins

