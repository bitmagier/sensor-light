use std::num::NonZeroU32;

use anyhow::Result;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::gpio::{InputPin, InterruptType, Level, OutputPin, Pin, PinDriver, Pull};
use esp_idf_svc::hal::prelude::Peripherals;
use esp_idf_svc::hal::task::notification::Notification;

const LED_POWER_STEPS: usize = 100;

// max. reaction delay when LED Power Phase is in Off or ON state
const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step/max-reaction delay when LED Power Phase is in PowerDown or PowerUp state
const LED_DIMM_STEP_DELAY_MS: u32 = 100;



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
struct BarState {
    pub ambient_light_sensor_lux: u32,
    pub phase: Phase,
    pub led_power_step: usize,
}

impl BarState {
    pub fn new() -> Self {
        BarState {
            ambient_light_sensor_lux: 0,
            phase: Phase::Off,
            led_power_step: 0,
        }
    }
    pub fn dark_enough_for_operation(&self) -> bool {
        // TODO determine active/passive state based on ambient light level history
        //  take medium of lowest 3 Lux values
        todo!()
    }
    
    pub fn duty_step_delay_ms(&self) -> u32 {
        match self.phase {
            Phase::Off | Phase::On => ON_OFF_REACTION_STEP_DELAY_MS,
            Phase::PowerDown | Phase::PowerUp => LED_DIMM_STEP_DELAY_MS
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

struct Device<P1: Pin> {
    pub presence_sensor_gpio_pin: PinDriver<'static, P1, gpio::Input>,
    pub notification: Notification
}

impl<P1: Pin> Device<P1> {
    pub fn read_sensors(&mut self, bar_state: &mut BarState) -> Result<()> {
        
        self.measure_ambient_light_level();
        
        if let Some(_) = self.notification.wait(esp_idf_svc::hal::delay::NON_BLOCK) {
            if bar_state.dark_enough_for_operation() {
                self.on_presence_sensor_event(bar_state);
                // re-enable interrupt after we have been triggering (disables the interrupt automatically)
                self.presence_sensor_gpio_pin.enable_interrupt()?;
            }
        }

        Ok(())
    }
    
    fn measure_ambient_light_level(&mut self) {
        // TODO measure ambient light level - only if LED is Off
        // + store level in ambient light level history buffer (past 10 sec)
        todo!()
    }
    
    fn on_presence_sensor_event(&mut self, bar_state: &mut BarState) {
        match self.presence_sensor_gpio_pin.get_level() {
            Level::Low => {
                bar_state.phase = Phase::PowerDown;
                log::debug!("Dimming down");
            }
            Level::High => {
                bar_state.phase = Phase::PowerUp;
                log::debug!("Dimming up");
            },
        }
    }

    pub fn apply_led_power_level(&self, _bar_state: &BarState) {
        // TODO LED PWM interface
        todo!()
    }
}


fn setup_presence_sensor_on_interrupt<P: InputPin + OutputPin>(
    gpio_pin: P
) -> Result<(PinDriver<'static, P, gpio::Input>, Notification)> {
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

    Ok((pin_driver, notification))
}

fn main() -> Result<()> {
    log::info!("on.");
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    
    let (presence_sensor_gpio_pin, presence_sensor_notification) = setup_presence_sensor_on_interrupt(peripherals.pins.gpio12)?;
    
    let mut device = Device {
        presence_sensor_gpio_pin,
        notification: presence_sensor_notification    
    };
    
    let mut bar_state = BarState::new();
    
    loop {
        FreeRtos::delay_ms(bar_state.duty_step_delay_ms());
        device.read_sensors(&mut bar_state)?;
        bar_state.calc_dimm_progress();
        device.apply_led_power_level(&bar_state);
    }
}

// TODO find a suitable project name (working project name "led-sensor-bar").
//  Candidates:
//  - smart-night-light
// TODO implement a faster dimm-up and slower dimm-down for the LED
// TODO disable unused pins