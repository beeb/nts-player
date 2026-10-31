use std::{
    mem,
    sync::{Mutex, MutexGuard},
    thread,
};

use anyhow::Result;
use esp_idf_svc::hal::{
    delay::{BLOCK, FreeRtos, TickType},
    gpio::{AnyIOPin, AnyOutputPin, Output, Pin, PinDriver, PinId},
    i2c::{I2C0, I2cConfig, I2cDriver},
    i2s::{
        I2S0, I2sDriver, I2sTx,
        config::{
            Config, DataBitWidth, SlotBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
            StdSlotConfig,
        },
    },
    units::Hertz,
};

use crate::{prelude::OrPanic, settings::MAX_VOLUME};

/// Time for the DAC's power-on reset after enabling its supply, in milliseconds.
///
/// This is 2 ticks at 100 Hz, because a 1 tick delay can end right away.
const POWER_ON_MS: u32 = 20;

/// Time for the soft mute to finish before stopping I2S, in milliseconds.
///
/// The mute takes 150 samples + 0.2 ms, about 3.6 ms at 44.1 kHz. See [`POWER_ON_MS`] for the tick rounding.
const MUTE_MS: u32 = 20;

/// Time for the PLL to lock after leaving standby, in milliseconds.
///
/// See [`POWER_ON_MS`] for the tick rounding.
const PLL_LOCK_MS: u32 = 20;

/// The DAC's I2C address.
const I2C_ADDR: u8 = 0x4C;

const I2C_BAUDRATE: Hertz = Hertz(100_000);

const I2C_TIMEOUT_MS: u64 = 50;

/// Standby register, 0x10 enters standby and 0 leaves it.
const REG_STANDBY: u8 = 2;

const REG_VOLUME_LEFT: u8 = 61;

const REG_VOLUME_RIGHT: u8 = 62;

/// Registers set in standby, for 44.1 kHz with BCK = 64 x fs and no SCK, as `[register, value]`.
///
/// The PLL and divider values come from table 131 of the datasheet (VREF mode).
const SETUP: [[u8; 2]; 13] = [
    [37, 0x1A], // ignore SCK errors and halt, manual clock dividers
    [4, 0x01],  // PLL on
    [13, 0x10], // PLL reference is BCK
    [20, 0],    // P = 1
    [21, 16],   // J = 16
    [22, 0],    // D = 0
    [23, 0],
    [24, 1],    // R = 2
    [27, 1],    // NMAC = 2
    [28, 15],   // NDAC = 16
    [29, 3],    // NCP = 4
    [30, 7],    // DOSR = 8
    [40, 0x03], // I2S format, 32-bit words
];

/// The only sample rate that [`SETUP`] supports.
const SETUP_RATE: u32 = 44_100;

/// Volume register value for 0 dB, each increment is 0.5 dB quieter.
const VOLUME_0_DB: u8 = 48;

const VOLUME_MUTE: u8 = 255;

/// Attenuation at [`MAX_VOLUME`].
const OUTPUT_LEVEL_DB: u8 = 3;

/// Attenuation of each volume step.
const STEP_DB: u8 = 3;

/// The DAC control pins and its I2S output.
pub struct DacPins {
    /// DAC supply enable, active high.
    pub en: AnyOutputPin<'static>,

    /// DAC soft mute, low is muted.
    pub xsmt: AnyOutputPin<'static>,

    /// The I2C peripheral.
    pub i2c: I2C0<'static>,

    /// I2C data pin.
    pub sda: AnyIOPin<'static>,

    /// I2C clock pin.
    pub scl: AnyIOPin<'static>,

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
    ///
    /// The DAC uses volume step `volume` once started.
    pub fn new(pins: DacPins, volume: u8) -> Result<Self> {
        let mut en = PinDriver::output(pins.en)?;
        en.set_low()?;
        let mut xsmt = PinDriver::output(pins.xsmt)?;
        xsmt.set_low()?;
        let config = I2cConfig::new()
            .baudrate(I2C_BAUDRATE)
            .sda_enable_pullup(true)
            .scl_enable_pullup(true);
        let i2c = I2cDriver::new(pins.i2c, pins.sda, pins.scl, &config)?;
        Ok(Self(Mutex::new(Inner {
            en,
            xsmt,
            i2c: Some(i2c),
            volume,
            i2s_gpios: [pins.i2s.bck.pin(), pins.i2s.din.pin(), pins.i2s.lrck.pin()],
            i2s_pins: Some(pins.i2s),
            i2s: None,
        })))
    }

    /// Power up and configure the DAC, start I2S at `rate` and unmute.
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
        if rate != SETUP_RATE {
            log::warn!("unsupported sample rate {rate} Hz");
        }
        let setup = d.setup();
        d.i2s = Some(pins.driver(rate)?);
        // keep streaming on failure, this also allows running without a DAC
        if let Err(e) = setup.and_then(|()| d.write_reg(REG_STANDBY, 0)) {
            log::warn!("could not set up the DAC: {e:?}");
        }
        FreeRtos::delay_ms(PLL_LOCK_MS);
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

    /// Set the volume step, from 0 (mute) to [`MAX_VOLUME`].
    pub fn set_volume(&self, step: u8) -> Result<()> {
        let mut d = self.lock();
        d.volume = step;
        if d.i2s.is_some() {
            d.write_volume()?;
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
        // I2C must not run without the DAC supply
        d.i2c = None;
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
    /// The I2C bus, until [`Dac::power_down`].
    i2c: Option<I2cDriver<'static>>,
    /// The current volume step.
    volume: u8,
    /// The I2S pins, until [`Dac::start`] uses them.
    i2s_pins: Option<I2sPins>,
    /// The running I2S output.
    i2s: Option<I2sDriver<'static, I2sTx>>,
    /// GPIO numbers of the I2S pins, to drive them low after the driver is gone.
    i2s_gpios: [PinId; 3],
}

impl Inner {
    /// Enter standby and write all the registers, which the DAC loses each time its supply turns off.
    fn setup(&mut self) -> Result<()> {
        self.write_reg(REG_STANDBY, 0x10)?;
        for [reg, value] in SETUP {
            self.write_reg(reg, value)?;
        }
        self.write_volume()
    }

    fn write_volume(&mut self) -> Result<()> {
        let value = self.volume_value();
        self.write_reg(REG_VOLUME_LEFT, value)?;
        self.write_reg(REG_VOLUME_RIGHT, value)
    }

    /// The volume register value for the current step.
    fn volume_value(&self) -> u8 {
        if self.volume == 0 {
            return VOLUME_MUTE;
        }
        let db = OUTPUT_LEVEL_DB + MAX_VOLUME.saturating_sub(self.volume) * STEP_DB;
        VOLUME_0_DB + 2 * db
    }

    fn write_reg(&mut self, reg: u8, value: u8) -> Result<()> {
        if let Some(i2c) = &mut self.i2c {
            let timeout = TickType::new_millis(I2C_TIMEOUT_MS).ticks();
            i2c.write(I2C_ADDR, &[reg, value], timeout)?;
        }
        Ok(())
    }
}

/// Block the calling thread forever.
fn halt() -> ! {
    loop {
        thread::park();
    }
}
