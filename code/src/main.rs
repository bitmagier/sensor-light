use anyhow::Result;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::gpio::{Level, Pin, PinDriver};
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::prelude::Peripherals;
use generic_array::typenum::U10;
use veml7700::Veml7700;

use crate::error::Error;
use crate::peripheral::{init_output_pin, init_presence_sensor_on_interrupt, init_veml7700, PresenceSensor};

mod error;
mod peripheral;


/// Number of stages the Led power level is increased from [Phase::Off] to [Phase::On] and vice versa.
const LED_POWER_STAGES: usize = 100;

/// max. reaction delay when LED Power Phase is in Off or ON state
const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step-delay (and also max. reaction time) when LED Power Phase is in PowerDown or PowerUp state
const LED_DIMM_DOWN_STEP_DELAY_MS: u32 = 200;

const LED_DIMM_UP_STEP_DELAY_MS: u32 = 65;

const LUX_THRESHOLD: u32 = 30;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Phase {
    Off,
    PowerDown,
    PowerUp,
    On,
}


// in range [0..LED_POWER_STAGES]
// translates to power level in range [0..100] via cubic curve y=x³
fn led_power_level(step: usize) -> usize {
    (step as f32 / 100_f32).powi(3).round() as usize
}

#[derive(Debug)]
struct State {
    // ambient light level history buffer (last 10 values)
    pub ambient_light_sensor_lux_buffer: median::stack::Filter<u32, U10>,
    pub phase: Phase,
    pub led_power_stage: usize,
}

impl State {
    pub fn new() -> Self {
        State {
            ambient_light_sensor_lux_buffer: median::stack::Filter::new(),
            phase: Phase::Off,
            led_power_stage: 0,
        }
    }

    pub fn is_dark_enough_for_operation(&self) -> bool {
        self.ambient_light_sensor_lux_buffer.len() > 0
            && self.ambient_light_sensor_lux_buffer.median() <= LUX_THRESHOLD
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
            Phase::Off => debug_assert_eq!(self.led_power_stage, 0),
            Phase::PowerDown => {
                if self.led_power_stage > 0 {
                    self.led_power_stage -= 1;
                    if self.led_power_stage == 0 {
                        self.phase = Phase::Off;
                    }
                }
            }
            Phase::PowerUp => {
                if self.led_power_stage < LED_POWER_STAGES {
                    self.led_power_stage += 1;
                    if self.led_power_stage == LED_POWER_STAGES {
                        self.phase = Phase::On;
                    }
                }
            }
            Phase::On => debug_assert_eq!(self.led_power_stage, LED_POWER_STAGES)
        }
    }
}


struct Devices<P1: Pin, P2: Pin> {
    pub presence_sensor: PresenceSensor<P1>,
    pub presence_sensor_power_pin: PinDriver<'static, P2, gpio::Output>,
    pub ambient_light_sensor: Veml7700<I2cDriver<'static>>,
}

impl<P1: Pin, P2: Pin> Devices<P1, P2> {
    pub fn read_sensors(&mut self, state: &mut State) -> Result<()> {
        if state.phase == Phase::Off {
            self.measure_ambient_light_level(state)?;
        }

        if self.presence_sensor.notification.wait(esp_idf_svc::hal::delay::NON_BLOCK).is_some() {
            if state.is_dark_enough_for_operation() {
                self.on_presence_sensor_event(state);
            }
            // re-enable interrupt after we have been triggering (disables the interrupt automatically)
            self.presence_sensor.gpio_pin.enable_interrupt()?;
        }

        Ok(())
    }

    // measure ambient light level - only if LED is Off
    fn measure_ambient_light_level(&mut self, state: &mut State) -> Result<()> {
        let lux: u32 = self.ambient_light_sensor.read_lux()
            .map_err(Error::from)?.round() as u32;
        state.ambient_light_sensor_lux_buffer.consume(lux);
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

    pub fn steer_presence_sensor(&mut self, state: &State) -> Result<()> {
        if state.is_dark_enough_for_operation() {
            self.enable_presence_sensor()?;
        } else {
            self.disable_presence_sensor()?;
        }
        Ok(())
    }

    pub fn enable_presence_sensor(&mut self) -> Result<()> {
        if !self.presence_sensor_power_pin.is_set_high() {
            self.presence_sensor_power_pin.set_high()?;
        }
        Ok(())
    }

    pub fn disable_presence_sensor(&mut self) -> Result<()> {
        if !self.presence_sensor_power_pin.is_set_low() {
            self.presence_sensor_power_pin.set_low()?;
        }
        Ok(())
    }

    pub fn apply_led_power_level(&self, _bar_state: &State) -> Result<()> {
        // TODO LED PWM interface
        todo!()
    }
}


fn main() -> Result<()> {
    log::info!("on.");
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();

    let mut devices = Devices {
        presence_sensor: init_presence_sensor_on_interrupt(peripherals.pins.gpio12)?,
        presence_sensor_power_pin: init_output_pin(peripherals.pins.gpio5)?,
        ambient_light_sensor: init_veml7700(
            peripherals.i2c0,
            peripherals.pins.gpio10,
            peripherals.pins.gpio11,
        )?,
    };

    let mut state = State::new();

    loop {
        FreeRtos::delay_ms(state.duty_step_delay_ms());
        devices.read_sensors(&mut state)?;
        state.calc_dimm_progress();
        devices.apply_led_power_level(&state)?;
        devices.steer_presence_sensor(&state)?;
    }
}


// TODO find a suitable project name (working project name "led-sensor-bar").
//  Candidates:
//  - floor-illuminator
//  - floor-light
//  - smart-night-light
// TODO disable unused pins

