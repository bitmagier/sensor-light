use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::prelude::Peripherals;
use log::info;
use rgb_led::{RGB8, WS2812RMT};

fn main() -> Result<()> {
    log::info!("On.");
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let _sysloop = EspSystemEventLoop::take()?;

    log::info!("Configuring onboard LED");
    
    // Onboard RGB LED pin
    // Rust ESP Board gpio2,  ESP32-C3-DevKitC-02 gpio8, ESP32-H2-DevKit-01 gpio8
    let led = peripherals.pins.gpio8;
    let channel = peripherals.rmt.channel0;
    let mut ws2812 = WS2812RMT::new(led, channel)?;
    
    loop {
        info!("Red!");
        ws2812.set_pixel(RGB8::new(255, 0, 0))?;
        FreeRtos::delay_ms(1000);
        info!("Green!");
        ws2812.set_pixel(RGB8::new(0, 255, 0))?;
        FreeRtos::delay_ms(1000);
        info!("Blue!");
        ws2812.set_pixel(RGB8::new(0, 0, 255))?;
        FreeRtos::delay_ms(1000);
    }
}
