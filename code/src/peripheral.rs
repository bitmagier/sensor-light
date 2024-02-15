use anyhow::Result;
use std::num::NonZeroU32;
use esp_idf_hal::gpio;
use esp_idf_hal::gpio::{InputPin, InterruptType, Output, OutputPin, Pin, PinDriver, Pull};
use esp_idf_hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::prelude::FromValueType;
use esp_idf_hal::task::notification::Notification;
use veml7700::Veml7700;
use crate::error::Error;

pub struct PresenceSensor<P1: Pin> {
    pub gpio_pin: PinDriver<'static, P1, gpio::Input>,
    pub notification: Notification,
}

pub fn init_presence_sensor_on_interrupt<P: InputPin + OutputPin>(
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
    veml7700_device.enable().map_err(|e| Error::from(e))?;
    Ok(veml7700_device)
}

pub fn init_output_pin<P: OutputPin>(pin: P) -> Result<PinDriver<'static, P, Output>> {
    let mut pin_driver = PinDriver::output(pin)?;
    pin_driver.set_low()?;
    Ok(pin_driver)
}