use std::{
    ptr,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use esp_idf_svc::{
    hal::{
        cpu::Core,
        delay::BLOCK,
        gpio::{AnyIOPin, AnyOutputPin},
        i2s::{
            I2S0, I2sDriver, I2sTx,
            config::{
                Config, DataBitWidth, SlotBitWidth, SlotMode, StdClkConfig, StdConfig,
                StdGpioConfig, StdSlotConfig,
            },
        },
        task::thread::ThreadSpawnConfiguration,
    },
    sys::uxTaskGetStackHighWaterMark,
};
use nanomp3::{DecodeError, Decoder, FrameInfo, MAX_SAMPLES_PER_FRAME};

use crate::buffer::RingBuffer;

/// Stack size of the decode thread (we use roughly 44 KB).
const STACK_SIZE: usize = 49 * 1024;

/// Amount of bytes pulled from the ring buffer into the read-ahead buffer.
const READAHEAD: usize = 16 * 1024;

const LOG_INTERVAL: Duration = Duration::from_secs(5);

/// The decode thread, playing MP3 from the ring buffer to I2S.
pub struct Player<'d> {
    /// The MP3 stream downloaded from the network.
    buffer: Arc<RingBuffer>,

    /// The nanomp3 decoder.
    decoder: &'d mut Decoder,

    /// The decoder needs a few frames to correctly identify the next frame.
    /// They are stored in this buffer.
    readahead: Vec<u8>,

    /// Pulse-code modulation (decoded) samples later sent to the DAC.
    pcm: Vec<i16>,

    /// To check the decoding throughput.
    stats: Stats,

    /// Generation of the ring buffer bytes in `readahead`.
    generation: u32,
}

impl<'d> Player<'d> {
    /// Start the decode thread on core 1, reading MP3 from `buffer` and writing PCM to I2S on `pins`.
    pub fn spawn(buffer: Arc<RingBuffer>, pins: I2sPins) -> Result<()> {
        ThreadSpawnConfiguration {
            name: Some(c"player"),
            pin_to_core: Some(Core::Core1),
            ..Default::default()
        }
        .set()?;
        let spawned = thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let mut decoder = Decoder::new();
                let mut player = Player::new(buffer, &mut decoder);
                if let Err(e) = player.run(pins) {
                    log::error!("player thread: {e:?}");
                }
            });
        ThreadSpawnConfiguration::default().set()?;
        spawned?;
        Ok(())
    }

    fn new(buffer: Arc<RingBuffer>, decoder: &'d mut Decoder) -> Self {
        Self {
            buffer,
            decoder,
            readahead: Vec::with_capacity(READAHEAD),
            pcm: vec![0; MAX_SAMPLES_PER_FRAME],
            stats: Stats::new(),
            generation: 0,
        }
    }

    fn run(&mut self, pins: I2sPins) -> Result<()> {
        self.buffer.wait_level(self.buffer.capacity() / 2);
        log::info!("buffer half full, playing");
        let first = self.next_frame();
        let mut rate = first.sample_rate;
        log::info!("{rate} Hz, starting I2S");
        let mut i2s = pins.driver(rate)?;
        let free = unsafe { uxTaskGetStackHighWaterMark(ptr::null_mut()) };
        log::info!("player stack: {free} bytes never used");
        let mut info = first;
        loop {
            let n_samples = info
                .samples_produced
                .saturating_mul(usize::from(info.channels.num()));
            let samples = self
                .pcm
                .get(..n_samples)
                .context("decoder produced more samples than the PCM buffer holds")?;
            // re-interpret the slice of i16 samples as a slice of u8 bytes (double the length)
            i2s.write_all(bytemuck::cast_slice(samples), BLOCK)?;
            info = self.next_frame();
            if info.sample_rate != rate {
                log::warn!("sample rate changed to {} Hz, ignored", info.sample_rate);
                // warn once per change, I2S keeps its first rate
                rate = info.sample_rate;
            }
        }
    }

    /// Decode frames into `pcm` until one produces audio.
    fn next_frame(&mut self) -> FrameInfo {
        loop {
            self.fill();
            let start = Instant::now();
            let (used, result) = self.decoder.decode(&self.readahead, &mut self.pcm);
            self.stats.record(start.elapsed(), &self.buffer);
            self.readahead.drain(..used.min(self.readahead.len()));
            match result {
                Ok(info) if info.samples_produced > 0 => return info,
                // normal at the start of a stream
                Ok(_) | Err(DecodeError::NoFrame | DecodeError::ReservoirUnavailable(_)) => {}
                Err(e) => log::warn!("skipped frame: {e}"),
            }
        }
    }

    /// Fill up the read-ahead buffer from the ring buffer, waiting for half full after an underrun or a station change.
    fn fill(&mut self) {
        while self.readahead.len() < READAHEAD {
            // an empty buffer after a station change is not an underrun
            if self.buffer.len() == 0 && self.buffer.generation() == self.generation {
                log::warn!("buffer underrun");
                self.buffer.wait_level(self.buffer.capacity() / 2);
                log::info!("buffer half full, playing");
            }
            let max = READAHEAD.saturating_sub(self.readahead.len());
            let start = self.readahead.len();
            let generation = self.buffer.read(&mut self.readahead, max);
            if generation != self.generation {
                // the bytes before `start` belong to the previous station
                self.generation = generation;
                self.readahead.drain(..start);
                mute();
                log::info!("station changed, refilling");
                self.buffer.wait_level(self.buffer.capacity() / 2);
                log::info!("buffer half full, playing");
                unmute();
            }
        }
    }
}

/// The I2S peripheral and pins for the player.
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
                .slot_bit_width(SlotBitWidth::Bits32),
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

/// Decode timing since the last log line.
struct Stats {
    frames: u32,
    total: Duration,
    max: Duration,
    since: Instant,
}

impl Stats {
    fn new() -> Self {
        Self {
            frames: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
            since: Instant::now(),
        }
    }

    fn record(&mut self, took: Duration, buffer: &RingBuffer) {
        self.frames += 1;
        self.total += took;
        self.max = self.max.max(took);
        if self.since.elapsed() >= LOG_INTERVAL {
            log::info!(
                "buffer: {} / {} bytes, {} frames, decode avg {:?}, max {:?}",
                buffer.len(),
                buffer.capacity(),
                self.frames,
                self.total / self.frames,
                self.max
            );
            *self = Self::new();
        }
    }
}

/// Silence the output. Does nothing until the DAC's XSMT pin is wired.
fn mute() {}

/// Undo [`mute`]. Does nothing until the DAC's XSMT pin is wired.
fn unmute() {}
