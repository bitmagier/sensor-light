#![feature(duration_constructors)]

use std::fmt::{Display, Formatter};
use std::ops::{Add, Sub};
use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_hal::ledc::LedcDriver;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::gpio::{Level, Pin, PinDriver};
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::hal::prelude::Peripherals;
use itertools::Itertools;
use ringbuffer::{AllocRingBuffer, RingBuffer};
use veml7700::Veml7700;

use crate::error::Error;
use crate::peripheral::{init_led_driver, init_output_pin, init_presence_sensor_on_interrupt, init_veml7700, PresenceSensor};

mod error;
mod peripheral;


/// Number of stages the Led power level is increased from [Phase::Off] to [Phase::On] and vice versa.
pub const LED_POWER_STAGES: usize = 100;

/// max. reaction delay when LED Power Phase is in Off or ON state
const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step-delay (and also max. reaction time) when LED Power Phase is in PowerDown or PowerUp state
const LED_DIMM_DOWN_STEP_DELAY_MS: u32 = 200;

const LED_DIMM_UP_STEP_DELAY_MS: u32 = 65;

const LUX_BUFFER_SIZE: usize = 10;
const LUX_THRESHOLD: u32 = 30;

const STATUS_LOG_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Phase {
    Off,
    PowerDown,
    PowerUp,
    On,
}


#[derive(Debug)]
struct State {
    // ambient light level history buffer (last 10 values)
    ambient_light_sensor_lux_buffer: AllocRingBuffer<u32>,
    pub phase: Phase,
    /// range: 0..LED_POWER_STAGES
    pub led_power_stage: usize,
}

impl State {
    pub fn new() -> Self {
        State {
            ambient_light_sensor_lux_buffer: AllocRingBuffer::new(LUX_BUFFER_SIZE),
            phase: Phase::Off,
            led_power_stage: 0,
        }
    }

    pub fn lex_level(&self) -> Option<u32> {
        if self.ambient_light_sensor_lux_buffer.is_empty() {
            None
        } else {
            let sorted = self.ambient_light_sensor_lux_buffer.iter()
                .sorted()
                .collect_vec();
            Some(
                *sorted[self.ambient_light_sensor_lux_buffer.len() / 2]
            )
        }
    }

    pub fn is_dark_enough_for_operation(&self) -> bool {
        match self.lex_level() {
            Some(lux) => lux <= LUX_THRESHOLD,
            None => false
        }
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

impl Display for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "dark_enough: {}, lux: {:?}, phase: {:?}, led_power_stage: {}",
               self.is_dark_enough_for_operation(),
               self.lex_level(),
               self.phase,
               self.led_power_stage
        )
    }
}

struct Devices<P1: Pin, P2: Pin> {
    presence_sensor: PresenceSensor<P1>,
    presence_sensor_power_pin: PinDriver<'static, P2, gpio::Output>,
    ambient_light_sensor: Veml7700<I2cDriver<'static>>,
    led_driver: LedcDriver<'static>,
    led_power_curve_scale_factor: f32,
}

impl<P1: Pin, P2: Pin> Devices<P1, P2> {
    pub fn new(
        presence_sensor: PresenceSensor<P1>,
        presence_sensor_power_pin: PinDriver<'static, P2, gpio::Output>,
        ambient_light_sensor: Veml7700<I2cDriver<'static>>,
        led_driver: LedcDriver<'static>,
    ) -> Self {
        let led_power_curve_scale_factor = Self::calc_led_power_curve_scale_factor(led_driver.get_max_duty());
        Self { presence_sensor, presence_sensor_power_pin, ambient_light_sensor, led_driver, led_power_curve_scale_factor }
    }
    pub fn read_sensors(&mut self, state: &mut State) -> Result<()> {
        if state.phase == Phase::Off {
            self.measure_ambient_light_level(state)?;
        }

        if self.presence_sensor.notification.wait(esp_idf_svc::hal::delay::NON_BLOCK).is_some() {
            if state.is_dark_enough_for_operation() {
                self.react_on_presence_sensor_event(state);
            }
            // re-enable interrupt after we have been triggering (disables the interrupt automatically)
            self.presence_sensor.gpio_pin.enable_interrupt()?;
        }

        Ok(())
    }

    // measure ambient light level - makes only sense to be called if LED is Off
    fn measure_ambient_light_level(&mut self, state: &mut State) -> Result<()> {
        let lux: u32 = self.ambient_light_sensor.read_lux()
            .map_err(Error::from)?.round() as u32;
        state.ambient_light_sensor_lux_buffer.push(lux);
        Ok(())
    }

    fn react_on_presence_sensor_event(&mut self, state: &mut State) {
        match self.presence_sensor.gpio_pin.get_level() {
            Level::Low => {
                state.phase = Phase::PowerDown;
                log::debug!("Powering down");
            }
            Level::High => {
                state.phase = Phase::PowerUp;
                log::debug!("Powering up");
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

    /// step comes in range [0..LED_POWER_STAGES]
    /// translates to power level in range [0..`max_duty`] via a logarithmic curve,
    /// scaled so that the highest step reaches `self.led_driver.get_max_duty()`
    fn calc_led_power_level(&self, step: usize) -> u32 {
        (Self::led_power_curve(step as u32) as f32 * self.led_power_curve_scale_factor).floor() as u32
    }


    fn calc_led_power_curve_scale_factor(max_duty: u32) -> f32 {
        Self::led_power_curve(max_duty) as f32 / (max_duty as f32)
    }

    pub fn apply_led_power_level(&mut self, bar_state: &State) -> Result<()> {
        let duty = self.calc_led_power_level(bar_state.led_power_stage);
        self.led_driver.set_duty(duty)?;
        Ok(())
    }

    // pure (unscaled) logarithmic curve `y = log(x+1) * z`
    fn led_power_curve(step: u32) -> u32 {
        (step + 1).ilog2()
    }
}


fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("on.");

    let peripherals = Peripherals::take().unwrap();

    let mut devices = Devices::new(
        init_presence_sensor_on_interrupt(peripherals.pins.gpio11)?,
        init_output_pin(peripherals.pins.gpio12)?,
        init_veml7700(
            peripherals.i2c0,
            peripherals.pins.gpio5,
            peripherals.pins.gpio4,
        )?,
        init_led_driver(
            peripherals.ledc.timer0,
            peripherals.ledc.channel0,
            peripherals.pins.gpio1,
        )?,
    );

    log::info!("initialized.");
    let mut state = State::new();
    let mut last_log_time = Instant::now().sub(Duration::from_mins(1));

    loop {
        log_status(&state, &mut last_log_time);
        FreeRtos::delay_ms(state.duty_step_delay_ms());
        devices.read_sensors(&mut state)?;
        state.calc_dimm_progress();
        devices.apply_led_power_level(&state)?;
        devices.steer_presence_sensor(&state)?;
    }
}

fn log_status(state: &State, last_log_time: &mut Instant) {
    let now = Instant::now();
    if last_log_time.add(STATUS_LOG_INTERVAL) <= now {
        *last_log_time = now;
        log::info!("{}", state)
    }
}

// TODO maybe disable unused pins

