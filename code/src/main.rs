#![feature(duration_constructors)]

use std::ops::Sub;
use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::prelude::Peripherals;

use crate::peripheral::{Devices, State};

mod error;
mod peripheral;

/// Number of stages (and also the maximum level) the LED power level is increased from [Phase::Off] to [Phase::On] and vice versa.
pub const LED_POWER_STAGES: u32 = 1000;

/// Percentage of hardware maximum LED brightness we want to reach
pub const LED_MAX_POWER_LEVEL_PERCENT: f32 = 0.08;

/// max. reaction delay when LED Power Phase is in Off or ON state
pub const ON_OFF_REACTION_STEP_DELAY_MS: u32 = 500;

// step-delay (and also max. reaction time) when LED Power Phase is in PowerDown or PowerUp state
pub const LED_DIM_DOWN_STEP_DELAY_MS: u32 = 14;

pub const LED_DIM_UP_STEP_DELAY_MS: u32 = 7;

pub const LUX_BUFFER_SIZE: usize = 8;
pub const LUX_THRESHOLD: f32 = 0.16;

const STATUS_LOG_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    // It is necessary to call this function once, otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // looks like we can't adjust the maximum loglevel (which is Info) as it seems to be hard-coded in EspLogger
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Starting up");

    let peripherals = Peripherals::take()?;

    log::info!("LED PWM OUT on GPIO 11");
    log::info!("VEML7700 ambient light sensor I2C: [SDA: GPIO 5, SCL: GPIO 4]");
    log::info!("Always-on switch (input) on GPIO 22");

    let mut devices = Devices::new(
        peripheral::init_presence_sensor(peripherals.pins.gpio1)?,
        peripheral::init_output_pin(peripherals.pins.gpio12)?,
        peripheral::init_veml7700(
            peripherals.i2c0,
            peripherals.pins.gpio5,
            peripherals.pins.gpio4,
        )?,
        peripheral::init_led_driver(
            peripherals.ledc.channel0,
            peripherals.ledc.timer0,
            peripherals.pins.gpio11,
        )?,
        peripheral::init_input_pin(peripherals.pins.gpio22)?
    );

    log::info!("LED maximum power level set to {:.0}%", 100.0 * LED_MAX_POWER_LEVEL_PERCENT);
    log::info!("Peripherals initialized");

    let mut state = State::new();
    let mut last_log_time = Instant::now().sub(Duration::from_mins(1));

    loop {
        peripheral::log_status(&state, &devices, &mut last_log_time);
        FreeRtos::delay_ms(state.duty_step_delay_ms());

        devices.read_sensors(&mut state)?;
        state.calc_dim_progress();
        devices.apply_led_power_level(&mut state)?;
        devices.steer_presence_sensor(&mut state)?;
    }
}
