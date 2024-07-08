//! Peripheral initialization

use anyhow::Result;
use esp_idf_hal::gpio;
use esp_idf_hal::gpio::{InputPin, Output, OutputPin, Pin, PinDriver, Pull};
use esp_idf_hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_hal::ledc::{LedcChannel, LedcDriver, LedcTimer, LedcTimerDriver, Resolution};
use esp_idf_hal::ledc::config::TimerConfig;
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::prelude::FromValueType;
use veml7700::Veml7700;

use crate::error::Error;

pub struct PresenceSensor<P1: Pin> {
    pub sensor_pin: PinDriver<'static, P1, gpio::Input>,
}

/// Init Radar presence sensor
pub fn init_presence_sensor<P: InputPin + OutputPin>(
    gpio_pin: P
) -> Result<PresenceSensor<P>> {

    // radar presence sensor
    let mut pin_driver = PinDriver::input(gpio_pin)?;
    pin_driver.set_pull(Pull::UpDown)?;

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
    veml7700_device.enable().map_err(Error::from)?;
    Ok(veml7700_device)
}

pub fn init_output_pin<P: OutputPin>(pin: P) -> Result<PinDriver<'static, P, Output>> {
    let mut pin_driver = PinDriver::output(pin)?;
    pin_driver.set_low()?;
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
    let timer_config = TimerConfig::default()
        .frequency(5000.Hz())
        .resolution(Resolution::Bits12);

    let timer_driver = LedcTimerDriver::new(timer, &timer_config)?;
    let mut driver = LedcDriver::new(channel, timer_driver, pin)?;
    driver.enable()?;
    Ok(driver)
}
