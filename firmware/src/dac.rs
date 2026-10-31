use std::{
    mem,
    sync::{Mutex, MutexGuard},
    thread,
};

use anyhow::Result;
use esp_idf_svc::hal::{
    delay::{BLOCK, FreeRtos},
    gpio::{AnyIOPin, AnyOutputPin, Output, Pin, PinDriver, PinId},
    i2s::{
        I2S0, I2sDriver, I2sTx,
        config::{
            Config, DataBitWidth, SlotBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
            StdSlotConfig,
        },
    },
};

use crate::prelude::OrPanic;

/// Time for the DAC's power-on reset after enabling its supply, in milliseconds.
///
/// This is 2 ticks at 100 Hz, because a 1 tick delay can end right away.
const POWER_ON_MS: u32 = 20;

/// Time for the soft mute to finish before stopping I2S, in milliseconds.
///
/// The mute takes 150 samples + 0.2 ms, about 3.6 ms at 44.1 kHz. See [`POWER_ON_MS`] for the tick rounding.
const MUTE_MS: u32 = 20;

/// The DAC control pins and its I2S output.
pub struct DacPins {
    /// DAC supply enable, active high.
    pub en: AnyOutputPin<'static>,

    /// DAC soft mute, low is muted.
    pub xsmt: AnyOutputPin<'static>,

    /// The I2S output.
    pub i2s: I2sPins,
}

/// The I2S peripheral and pins for the DAC.
pub struct I2sPins {
    /// Inter-IC Sound device.
    pub i2s: I2S0<'static>,

    /// Bit clock pin.
    pub bck: AnyIOPin<'static>,

    /// Data pin.
    pub din: AnyOutputPin<'static>,

    /// Word select pin.
    pub lrck: AnyIOPin<'static>,
}

impl I2sPins {
    /// Open the I2S output at `rate`: 16-bit stereo samples in 32-bit slots, so BCK = 64 x fs.
    fn driver(self, rate: u32) -> Result<I2sDriver<'static, I2sTx>> {
        let config = StdConfig::new(
            // send silence on underrun instead of repeating the last DMA buffer
            Config::new().auto_clear(true),
            StdClkConfig::from_sample_rate_hz(rate),
            StdSlotConfig::philips_slot_default(DataBitWidth::Bits16, SlotMode::Stereo)
                .slot_bit_width(SlotBitWidth::Bits32)
                // the default follows the 16-bit samples, which skews the LRCK duty cycle
                .ws_width(32),
            StdGpioConfig::default(),
        );
        let mut i2s = I2sDriver::new_std_tx(
            self.i2s,
            &config,
            self.bck,
            self.din,
            AnyIOPin::none(),
            self.lrck,
        )?;
        i2s.tx_enable()?;
        Ok(i2s)
    }
}

/// The DAC with its I2S output, shared by the player and the input threads.
pub struct Dac(Mutex<Inner>);

impl Dac {
    /// Take the control pins, leaving the DAC unpowered and muted.
    pub fn new(pins: DacPins) -> Result<Self> {
        let mut en = PinDriver::output(pins.en)?;
        en.set_low()?;
        let mut xsmt = PinDriver::output(pins.xsmt)?;
        xsmt.set_low()?;
        Ok(Self(Mutex::new(Inner {
            en,
            xsmt,
            i2s_gpios: [pins.i2s.bck.pin(), pins.i2s.din.pin(), pins.i2s.lrck.pin()],
            i2s_pins: Some(pins.i2s),
            i2s: None,
        })))
    }

    /// Power up the DAC, start I2S at `rate` and unmute.
    ///
    /// Blocks forever after [`Dac::power_down`].
    pub fn start(&self, rate: u32) -> Result<()> {
        let mut d = self.lock();
        let Some(pins) = d.i2s_pins.take() else {
            drop(d);
            halt()
        };
        d.en.set_high()?;
        FreeRtos::delay_ms(POWER_ON_MS);
        d.i2s = Some(pins.driver(rate)?);
        d.xsmt.set_high()?;
        log::info!("DAC on, {rate} Hz");
        Ok(())
    }

    /// Write interleaved samples to I2S, blocking until the DMA buffers reads
    /// them.
    ///
    /// Blocks forever after [`Dac::power_down`].
    pub fn write(&self, samples: &[u8]) -> Result<()> {
        let mut d = self.lock();
        let Some(i2s) = &mut d.i2s else {
            drop(d);
            halt()
        };
        i2s.write_all(samples, BLOCK)?;
        Ok(())
    }

    pub fn mute(&self) -> Result<()> {
        self.lock().xsmt.set_low()?;
        Ok(())
    }

    /// Undo [`Dac::mute`], if the DAC is running.
    pub fn unmute(&self) -> Result<()> {
        let mut d = self.lock();
        if d.i2s.is_some() {
            d.xsmt.set_high()?;
        }
        Ok(())
    }

    /// Mute, stop I2S and drive its pins low, then cut the DAC supply.
    ///
    /// The DAC can't start again until a reboot.
    pub fn power_down(&self) -> Result<()> {
        let mut d = self.lock();
        d.xsmt.set_low()?;
        FreeRtos::delay_ms(MUTE_MS);
        // dropping the driver or the pins releases the I2S GPIOs
        d.i2s = None;
        d.i2s_pins = None;
        for gpio in d.i2s_gpios {
            let mut pin = PinDriver::output(unsafe { AnyOutputPin::steal(gpio) })?;
            pin.set_low()?;
            // dropping the driver would reset the pin
            mem::forget(pin);
        }
        d.en.set_low()?;
        log::info!("DAC off");
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().or_panic("DAC mutex poisoned")
    }
}

struct Inner {
    en: PinDriver<'static, Output>,
    xsmt: PinDriver<'static, Output>,
    /// The I2S pins, until [`Dac::start`] uses them.
    i2s_pins: Option<I2sPins>,
    /// The running I2S output.
    i2s: Option<I2sDriver<'static, I2sTx>>,
    /// GPIO numbers of the I2S pins, to drive them low after the driver is gone.
    i2s_gpios: [PinId; 3],
}

/// Block the calling thread forever.
fn halt() -> ! {
    loop {
        thread::park();
    }
}
