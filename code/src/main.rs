use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::RwLock;
use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{InterruptType, Level, PinDriver, Pull};
use esp_idf_svc::hal::prelude::Peripherals;

const LED_MAX_POWER_LEVEL: usize = 100;
enum Phase {
    Off,
    PowerDown,
    PowerUp,
    On
}

struct BarState {
    dimm_phase: Phase,
    led_power_level: usize,
}

impl Default for BarState {
    fn default() -> Self {
        BarState {
            dimm_phase: Phase::Off,
            led_power_level: 0,
        }
    }
}



fn interrupt_read() -> Result<()> {
    let peripherals = Peripherals::take()?;
    let mut pin = PinDriver::input(peripherals.pins.gpio12)?;
    pin.set_pull(Pull::Floating)?;

    log::info!("Radar Sensor - Interrupt Hello, world!");

    static mut BAR_STATE: RwLock<BarState> = RwLock::new(BarState::default());

    fn enter_dimm_out_phase() {
        log::info!("DIMM out ..");
    }

    fn enter_dimm_on_phase() {
        log::info!("DIMM on ..");
    }

    fn proceed_dimming() {
        
        let phase = LED_BAR_PHASE.load(Ordering::Relaxed);
        match phase.as {
            Phase::Off => assert_eq!(LED_POWER_LEVEL, 0),
            Phase::PowerDown => {

            }
            Phase::PowerUp => {}
            Phase::On => assert_eq!(LED_POWER_LEVEL, LED_MAX_POWER_LEVEL)
        }
    }

    let callback = || {
        let level = pin.get_level();
        match level {
            Level::Low => enter_dimm_out_phase(),
            Level::High => enter_dimm_on_phase(),
        }
    };

    pin.set_interrupt_type(InterruptType::AnyEdge)?;
    unsafe { pin.subscribe(callback)? }
    pin.enable_interrupt()?;

    loop {
        FreeRtos::delay_ms(500);
        log::info!("power level (0..10): {}", LED_POWER_LEVEL.load(Ordering::Relaxed))
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
    let _sysloop = EspSystemEventLoop::take()?;

    loop {

        FreeRtos::delay_ms(1000);
    }
}
