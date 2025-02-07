//! Peripheral initialization & control

use std::fmt::{Display, Formatter};
use std::ops::Add;
use std::time::Instant;

use anyhow::Result;
use esp_idf_hal::gpio;
use esp_idf_hal::gpio::{InputPin, Level, OutputPin, Pin, PinDriver, Pull};
use esp_idf_hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_hal::ledc::{LedcChannel, LedcDriver, LedcTimer, LedcTimerDriver, Resolution};
use esp_idf_hal::ledc::config::TimerConfig;
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::prelude::FromValueType;
use itertools::Itertools;
use ringbuffer::{AllocRingBuffer, RingBuffer};
use veml7700::{PowerSavingMode, Veml7700};

use crate::{LED_DIM_DOWN_STEP_DELAY_MS, LED_DIM_UP_STEP_DELAY_MS, LED_MAX_POWER_LEVEL_PERCENT, LED_POWER_STAGES, LUX_BUFFER_SIZE, LUX_THRESHOLD, ON_OFF_REACTION_STEP_DELAY_MS, STATUS_LOG_INTERVAL};
use crate::error::Error;

pub struct PresenceSensor<P1: Pin> {
    pub sensor_pin: PinDriver<'static, P1, gpio::Input>,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Phase {
    Off,
    PowerDown,
    PowerUp,
    On,
}

#[derive(Debug)]
pub struct State {
    // ambient light level history buffer (last 10 values)
    ambient_light_sensor_lux_buffer: AllocRingBuffer<f32>,
    pub phase: Phase,
    /// range: 0..LED_POWER_STAGES
    pub led_power_stage: u32,
    pub duty: u32,
    pub light_always_on: bool
}

impl State {
    pub fn new() -> Self {
        State {
            ambient_light_sensor_lux_buffer: AllocRingBuffer::new(LUX_BUFFER_SIZE),
            phase: Phase::Off,
            led_power_stage: 0,
            duty: 0,
            light_always_on: false
        }
    }

    pub fn lux_level(&self) -> Option<f32> {
        if self.ambient_light_sensor_lux_buffer.is_empty() {
            None
        } else {
            let sorted = self.ambient_light_sensor_lux_buffer.iter()
                .sorted_by(|a,b | a.partial_cmp(b).unwrap())
                .collect_vec();
            Some(
                *sorted[sorted.len() / 2]
            )
        }
    }

    pub fn is_dark_enough_for_operation(&self) -> bool {
        match self.lux_level() {
            Some(lux) => lux <= LUX_THRESHOLD,
            None => false
        }
    }

    pub fn duty_step_delay_ms(&self) -> u32 {
        match self.phase {
            Phase::Off | Phase::On => ON_OFF_REACTION_STEP_DELAY_MS,
            Phase::PowerDown => LED_DIM_DOWN_STEP_DELAY_MS,
            Phase::PowerUp => LED_DIM_UP_STEP_DELAY_MS
        }
    }

    pub fn calc_dim_progress(&mut self) {
        match self.phase {
            Phase::Off => {
                self.led_power_stage = 0
            },
            Phase::PowerDown => {
                if self.led_power_stage > 0 {
                    self.led_power_stage -= 1;
                }
                if self.led_power_stage == 0 {
                    self.phase = Phase::Off;
                }
            }
            Phase::PowerUp => {
                if self.led_power_stage < LED_POWER_STAGES {
                    self.led_power_stage += 1;
                }
                if self.led_power_stage == LED_POWER_STAGES {
                    self.phase = Phase::On;
                }
            }
            Phase::On => {
                self.led_power_stage = LED_POWER_STAGES
            }
        }
    }
}

impl Display for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "logic state: dark_enough: {}, lux: {:?}, phase: {:?}, led_power_stage: {:.0}%",
               self.is_dark_enough_for_operation(),
               self.lux_level(),
               self.phase,
               100.0 * self.led_power_stage as f32 / LED_POWER_STAGES as f32
        )
    }
}

pub struct Devices<P1: Pin, P2: Pin, P3: Pin> {
    presence_sensor: PresenceSensor<P1>,
    presence_sensor_power_pin: PinDriver<'static, P2, gpio::Output>,
    ambient_light_sensor: Veml7700<I2cDriver<'static>>,
    led_driver: LedcDriver<'static>,
    led_power_curve_scale_factor: f32,
    light_always_on_switch_pin: PinDriver<'static, P3, gpio::Input>
}

impl<P1: Pin, P2: Pin, P3: Pin> Devices<P1, P2, P3> {
    pub fn new(
        presence_sensor: PresenceSensor<P1>,
        presence_sensor_power_pin: PinDriver<'static, P2, gpio::Output>,
        ambient_light_sensor: Veml7700<I2cDriver<'static>>,
        led_driver: LedcDriver<'static>,
        light_always_on_switch_pin: PinDriver<'static, P3, gpio::Input>
    ) -> Self {
        log::info!("Presence sensor power switch OUT on GPIO {}", presence_sensor_power_pin.pin());

        let led_power_curve_scale_factor = Self::calc_led_power_curve_scale_factor(led_driver.get_max_duty());
        log::info!("LED power curve scale factor: {}", led_power_curve_scale_factor);
        Self {
            presence_sensor,
            presence_sensor_power_pin,
            ambient_light_sensor,
            led_driver,
            led_power_curve_scale_factor,
            light_always_on_switch_pin
        }
    }

    pub fn read_sensors(&mut self, state: &mut State) -> Result<()> {
        state.light_always_on = self.light_always_on_switch_pin.is_high();

        if state.phase == Phase::Off {
            self.measure_ambient_light_level(state)?;
        }

        if state.light_always_on {
            state.phase = Phase::On
        } else {
            self.read_presence_sensor_and_apply_phase(state);
        }

        Ok(())
    }

    // measure ambient light level - makes only sense to be called if LED is Off
    fn measure_ambient_light_level(&mut self, state: &mut State) -> Result<()> {
        let lux: f32 = self.ambient_light_sensor.read_lux()
            .map_err(Error::from)?;
        state.ambient_light_sensor_lux_buffer.push(lux);
        Ok(())
    }

    fn read_presence_sensor_and_apply_phase(&mut self, state: &mut State) {
        match self.presence_sensor.sensor_pin.get_level() {
            Level::Low => {
                if state.phase != Phase::Off
                    && state.phase != Phase::PowerDown
                {
                    state.phase = Phase::PowerDown;
                    log::info!("Powering down");
                }
            }
            Level::High => {
                if state.is_dark_enough_for_operation()
                    && state.phase != Phase::On
                    && state.phase != Phase::PowerUp
                {
                    state.phase = Phase::PowerUp;
                    log::info!("Powering up");
                }
            }
        }
    }

    pub fn steer_presence_sensor(&mut self, state: &mut State) -> Result<()> {
        match (
            state.light_always_on,
            state.phase,
            state.is_dark_enough_for_operation())
        {
            (true, _, _) |
            (false, Phase::Off, false) => self.disable_presence_sensor()?,
            _ => self.enable_presence_sensor()?
        }
        Ok(())
    }

    fn enable_presence_sensor(&mut self) -> Result<()> {
        self.presence_sensor_power_pin.set_high()?;
        Ok(())
    }

    fn disable_presence_sensor(&mut self) -> Result<()> {
        self.presence_sensor_power_pin.set_low()?;
        Ok(())
    }

    pub fn presence_sensor_enabled(&self) -> bool {
        self.presence_sensor_power_pin.is_set_high()
    }

    pub fn apply_led_power_level(&mut self, bar_state: &mut State) -> Result<()> {
        bar_state.duty = self.calc_led_power_level(bar_state.led_power_stage);

        // We are using a gate driver circuit to feed the PWM signal via a NPN-Transistor to a N-channel MOSFET.
        // Because of the nature of that circuit we need to invert our signal.
        // (MOSFET's gate will be open when we have our IO pin on low).
        let inverted_duty = self.led_driver.get_max_duty() - bar_state.duty;

        self.led_driver.set_duty(inverted_duty)?;
        Ok(())
    }

    /// Step comes in range [0..LED_MAX_POWER_STAGE]
    /// translates to power level in range [0..`max_duty`] via a logarithmic curve,
    /// scaled so that the highest step reaches `self.led_driver.get_max_duty()`
    /// ```
    /// y - duty
    /// x - power stage [0..LED_MAX_POWER_STAGE]
    /// z - scale factor to reach LED driver max_duty when we are at 100%
    /// ```
    fn calc_led_power_level(&self, power_stage: u32) -> u32 {
        (Self::led_power_curve(power_stage) * self.led_power_curve_scale_factor).round() as u32
    }

    fn calc_led_power_curve_scale_factor(led_driver_max_duty: u32) -> f32 {
        (led_driver_max_duty as f32 * LED_MAX_POWER_LEVEL_PERCENT) / (Self::led_power_curve(LED_POWER_STAGES))
    }

    // pure (unscaled) logarithmic curve
    fn led_power_curve(power_stage: u32) -> f32 {
        f32::ln((power_stage as f32) / 50.0 + 1.0)
    }
}

pub fn log_status<P1: Pin, P2: Pin, P3: Pin>(state: &State, devices: &Devices<P1, P2, P3>, last_log_time: &mut Instant) {
    let now = Instant::now();
    if last_log_time.add(STATUS_LOG_INTERVAL) <= now {
        *last_log_time = now;
        log::info!("{} | Hardware: Chip PWM duty: {}/{}, Presence sensor: (enabled: {}, signal: {:?}), Always-on-mode: {:?}",
            state,
            devices.led_driver.get_duty(),
            devices.led_driver.get_max_duty(),
            devices.presence_sensor_enabled(),
            devices.presence_sensor.sensor_pin.get_level(),
            match devices.light_always_on_switch_pin.get_level() {
                Level::Low => "No",
                Level::High => "Yes"
            }
        )
    }
}

/// Init Radar presence sensor
pub fn init_presence_sensor<P: InputPin + OutputPin>(
    gpio_pin: P
) -> Result<PresenceSensor<P>> {
    log::info!("Presence sensor IN on GPIO {}", gpio_pin.pin());
    // radar presence sensor
    let mut pin_driver = PinDriver::input(gpio_pin)?;
    pin_driver.set_pull(Pull::Down)?;

    Ok(PresenceSensor {
        sensor_pin: pin_driver
    })
}

pub fn init_veml7700<I2C: I2c>(
    i2c: impl Peripheral<P=I2C> + 'static,
    sda: impl Peripheral<P=impl InputPin + OutputPin> + 'static,
    scl: impl Peripheral<P=impl InputPin + OutputPin> + 'static,
) -> Result<Veml7700<I2cDriver<'static>>>
{
    let config = I2cConfig::new().baudrate(100.kHz().into());

    let i2c_driver = I2cDriver::new(i2c, sda, scl, &config)?;

    // Initialize the VEML7700 with I2C
    let mut veml7700_device = Veml7700::new(i2c_driver);
    // PSM mode Three (in combination with defaults for other settings) means a refresh time of 2.1 sec
    veml7700_device.enable_power_saving(PowerSavingMode::Three).map_err(Error::from)?;
    veml7700_device.enable().map_err(Error::from)?;
    Ok(veml7700_device)
}

pub fn init_output_pin<P: OutputPin>(pin: P) -> Result<PinDriver<'static, P, gpio::Output>> {
    let mut pin_driver = PinDriver::output(pin)?;
    pin_driver.set_low()?;
    Ok(pin_driver)
}

pub fn init_input_pin<P: InputPin + OutputPin>(pin: P) -> Result<PinDriver<'static, P, gpio::Input>> {
    let mut pin_driver = PinDriver::input(pin)?;
    pin_driver.set_pull(Pull::Down)?;
    Ok(pin_driver)
}

pub fn init_led_driver<C, T>(
    channel: impl Peripheral<P=C> + 'static,
    timer: impl Peripheral<P=T> + 'static,
    pin: impl Peripheral<P=impl OutputPin> + 'static,
) -> Result<LedcDriver<'static>>
where
    C: LedcChannel<SpeedMode=<T as LedcTimer>::SpeedMode>,
    T: LedcTimer + 'static,
{
    let freq = 4321.Hz();
    let resolution = Resolution::Bits12;

    let timer_config = TimerConfig::default()
        .frequency(freq.into())
        .resolution(resolution);

    let timer_driver = LedcTimerDriver::new(timer, &timer_config)?;
    let mut driver = LedcDriver::new(channel, timer_driver, pin)?;
    driver.enable()?;
    Ok(driver)
}
