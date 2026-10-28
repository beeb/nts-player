use std::sync::Mutex;

use anyhow::Result;
use esp_idf_svc::hal::{
    gpio::{Output, OutputPin, PinDriver},
    rmt::config::{MemoryAccess, TxChannelConfig},
    units::Hertz,
};
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use crate::{
    prelude::OrPanic,
    stream::{LIVE_STATIONS, STATION_COUNT},
};

/// Number of LEDs on the ring, one per station.
const RING_LEDS: u8 = STATION_COUNT;

/// Index of the bottom LED, where the volume bar starts.
const VOLUME_START: u8 = RING_LEDS / 2;

/// Level of the bottom LED at volume 0, as a fraction of the volume color out of 255.
const VOLUME_ZERO: u8 = 128;

/// Brightness of the lit ring LEDs.
const BRIGHTNESS: u8 = 16;

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
    pub const LIVE: Self = Self {
        r: BRIGHTNESS - BRIGHTNESS / 3,
        g: BRIGHTNESS - BRIGHTNESS / 3,
        b: BRIGHTNESS - BRIGHTNESS / 3,
    };
    pub const MIXTAPE: Self = Self {
        r: BRIGHTNESS,
        g: 0,
        b: 0,
    };
    pub const VOLUME: Self = Self {
        r: BRIGHTNESS - BRIGHTNESS / 3,
        g: BRIGHTNESS / 2,
        b: 0,
    };
    pub const WIFI: Self = Self {
        r: 0,
        g: 0,
        b: BRIGHTNESS,
    };
    pub const ERROR: Self = Self {
        r: BRIGHTNESS,
        g: 0,
        b: 0,
    };

    /// Color of a station's LED, white for the live channels and red for the mixtapes.
    pub fn station(index: u8) -> Self {
        if index < LIVE_STATIONS {
            Self::LIVE
        } else {
            Self::MIXTAPE
        }
    }

    /// Scale the color by `level / 255`.
    pub fn scale(self, level: u8) -> Self {
        // round to nearest, flooring skews the hue at low levels
        let f =
            |c: u8| u8::try_from((u16::from(c) * u16::from(level) + 127) / 255).unwrap_or(u8::MAX);
        Self {
            r: f(self.r),
            g: f(self.g),
            b: f(self.b),
        }
    }
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

/// The ring of WS2812 LEDs around the knob, with its power switch.
pub struct Ring {
    driver: Ws2812Esp32RmtDriver<'static>,
    power: PinDriver<'static, Output>,
    /// Scale of all colors, out of 255.
    level: u8,
}

impl Ring {
    /// Power the ring on and turn all its LEDs off.
    pub fn new(data: impl OutputPin + 'static, power: impl OutputPin + 'static) -> Result<Self> {
        // a DMA buffer fits all pixels at once
        // esp-idf-hal 0.47 panics in the ISR when the RMT memory fills up mid-transfer
        let config = TxChannelConfig {
            resolution: Hertz(10_000_000),
            memory_access: MemoryAccess::Direct {
                memory_block_symbols: 1024,
            },
            ..Default::default()
        };
        let mut power = PinDriver::output(power)?;
        power.set_high()?;
        let mut ring = Self {
            driver: Ws2812Esp32RmtDriver::new_with_config(data, &config)?,
            power,
            level: u8::MAX,
        };
        ring.off()?;
        Ok(ring)
    }

    /// Light the LED at `index` only.
    pub fn dot(&mut self, index: u8, color: Color) -> Result<()> {
        self.write(color, |i| i == index)
    }

    /// Light all LEDs.
    pub fn all(&mut self, color: Color) -> Result<()> {
        self.write(color, |_| true)
    }

    /// Light a bar of `volume` LEDs clockwise from the bottom, or only a dim bottom LED at 0.
    pub fn volume(&mut self, volume: u8) -> Result<()> {
        // a dim marker shows that volume mode is on
        if volume == 0 {
            return self.dot(VOLUME_START, Color::VOLUME.scale(VOLUME_ZERO));
        }
        self.write(Color::VOLUME, |i| {
            (i + RING_LEDS - VOLUME_START) % RING_LEDS < volume
        })
    }

    /// Scale all following writes by `level / 255`, to dim the whole ring.
    pub fn set_level(&mut self, level: u8) {
        self.level = level;
    }

    /// Turn all LEDs off and cut the ring power.
    pub fn shutdown(&mut self) -> Result<()> {
        self.off()?;
        self.power.set_low()?;
        Ok(())
    }

    fn off(&mut self) -> Result<()> {
        self.write(Color::OFF, |_| false)
    }

    /// Write `color` to the LEDs for which `lit` is true, and turn the others off.
    fn write(&mut self, color: Color, lit: impl Fn(u8) -> bool) -> Result<()> {
        // the ring expects GRB order
        let pixels = (0..RING_LEDS).flat_map(|i| {
            let c = if lit(i) {
                color.scale(self.level)
            } else {
                Color::OFF
            };
            [c.g, c.r, c.b]
        });
        self.driver.write_blocking(pixels)?;
        Ok(())
    }
}
