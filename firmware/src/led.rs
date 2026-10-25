use std::sync::Mutex;

use anyhow::Result;
use esp_idf_svc::hal::gpio::OutputPin;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use crate::prelude::OrPanic;

/// Color of an RGB LED.
#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    // debug LED (onboard the waveshare dev board) can be super dim
    pub const OFF: Self = Self { r: 0, g: 0, b: 0 };
    pub const DBG_RED: Self = Self { r: 1, g: 0, b: 0 };
    pub const DBG_YELLOW: Self = Self { r: 1, g: 1, b: 0 };
    pub const DBG_GREEN: Self = Self { r: 0, g: 1, b: 0 };
    pub const DBG_BLUE: Self = Self { r: 0, g: 0, b: 1 };
}

/// A WS2812 RGB LED on a data pin.
pub struct Led(Mutex<Ws2812Esp32RmtDriver<'static>>);

impl Led {
    pub fn new(pin: impl OutputPin + 'static) -> Result<Self> {
        Ok(Self(Mutex::new(Ws2812Esp32RmtDriver::new(pin)?)))
    }

    pub fn set(&self, color: Color) -> Result<()> {
        // the onboard LED expects RGB order
        self.0
            .lock()
            .or_panic("LED mutex poisoned")
            .write_blocking([color.r, color.g, color.b].into_iter())?;
        Ok(())
    }
}
