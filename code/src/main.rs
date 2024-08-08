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
use crate::devices::{Devices, State};
use crate::error::Error;
use crate::peripheral::{init_led_driver, init_output_pin, init_presence_sensor, init_veml7700, PresenceSensor};

mod error;
mod peripheral;
mod devices;

/// Number of stages the Led power level is increased from [Phase::Off] to [Phase::On] and vice versa.
pub const LED_POWER_STAGES: u32 = 1000;

/// Percentage of hardware maximum LED brightness we want to reach
pub const LED_MAX_POWER_LEVEL_PERCENT: f32 = 0.45; 

/// max. reaction delay when LED Power Phase is in Off or ON state
pub const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step-delay (and also max. reaction time) when LED Power Phase is in PowerDown or PowerUp state
pub const LED_DIMM_DOWN_STEP_DELAY_MS: u32 = 10;

pub const LED_DIMM_UP_STEP_DELAY_MS: u32 = 7;

pub const LUX_BUFFER_SIZE: usize = 10;
pub const LUX_THRESHOLD: u32 = 12;

const STATUS_LOG_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    // It is necessary to call this function once, otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // looks like we can't adjust the maximum loglevel (which is Info) as it seems to be hard-coded in EspLogger 
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Starting up");

    let peripherals = Peripherals::take().unwrap();

    log::info!("LED PWM OUT on GPIO 11");
    log::info!("VEML7700 ambient light sensor I2C: [SDA: GPIO 4, SCL: GPIO 5]");

    let mut devices = Devices::new(
        init_presence_sensor(peripherals.pins.gpio1)?,
        init_output_pin(peripherals.pins.gpio12)?,
        init_veml7700(
            peripherals.i2c0,
            peripherals.pins.gpio4,
            peripherals.pins.gpio5,
        )?,
        init_led_driver(
            peripherals.ledc.channel0,
            peripherals.ledc.timer0,
            peripherals.pins.gpio11,
        )?,
    );

    log::info!("LED maximum power level set to {:.0}%", 100.0 * LED_MAX_POWER_LEVEL_PERCENT);
    log::info!("Peripherals initialized");
    
    let mut state = State::new();
    let mut last_log_time = Instant::now().sub(Duration::from_mins(1));

    loop {
        log_status(&state, &devices, &mut last_log_time);
        FreeRtos::delay_ms(state.duty_step_delay_ms());
        
        devices.read_sensors(&mut state)?;
        state.calc_dimm_progress();
        devices.apply_led_power_level(&mut state)?;
        devices.steer_presence_sensor(&mut state)?;
    }
}