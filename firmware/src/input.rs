use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use esp_idf_svc::{
    hal::{
        delay::FreeRtos,
        gpio::{AnyInputPin, Input, PinDriver, Pull},
    },
    nvs::EspDefaultNvsPartition,
    sys::{
        esp, esp_deep_sleep_start, esp_sleep_enable_ext0_wakeup, esp_wifi_stop, rtc_gpio_pullup_en,
    },
};

use crate::{
    buffer::RingBuffer,
    led::{Color, Led, Ring},
    settings::{MAX_VOLUME, Settings},
    stream::{STATION_COUNT, Tuner},
};

/// Stack size of the input thread, NVS writes need more than the 4 KB default.
const STACK_SIZE: usize = 8 * 1024;

/// Hold time for putting the device to sleep.
const LONG_PRESS: Duration = Duration::from_millis(700);

/// Time after the last step before a change is applied and saved.
const SETTLE: Duration = Duration::from_millis(600);

/// Polling interval for the inputs, in milliseconds.
const POLL_DELAY_MS: u32 = 10;

/// Number of [`POLL_DELAY_MS`] duration ticks for the switch debounce.
const DEBOUNCE_TICKS: u32 = 3;

/// Period of the station LED pulse while buffering.
const PULSE_PERIOD: Duration = Duration::from_millis(1500);

/// Lowest pulse level, as a fraction of the station color out of 255.
const PULSE_MIN: u8 = 50;

/// The input pins, all active low.
pub struct InputPins {
    /// Step -1 (joystick left on the breadboard, encoder A later).
    pub left: AnyInputPin<'static>,

    /// Step +1 (joystick right on the breadboard, encoder B later).
    pub right: AnyInputPin<'static>,

    /// Push button for mode switch and deep sleep/wake up.
    pub push: AnyInputPin<'static>,
}

/// The input and UI thread state.
pub struct Ui {
    left: Switch,
    right: Switch,
    push: Switch,
    led: Arc<Led>,
    ring: Ring,
    tuner: Arc<Tuner>,
    buffer: Arc<RingBuffer>,
    nvs: EspDefaultNvsPartition,
    settings: Settings,
    mode: Mode,
    /// Whether the LEDs were pulsing during the last draw.
    pulsing: bool,
    /// Start of the pulse animation.
    epoch: Instant,
    /// Start of the current push.
    ///
    /// The first push (waking up the device) does not count, so it's still `None`. Also `None` when the button is
    /// not pressed.
    pushed_at: Option<Instant>,
    /// Time of the last station or volume change that is not applied yet.
    changed_at: Option<Instant>,
}

impl Ui {
    /// Start the input thread.
    pub fn spawn(
        pins: InputPins,
        led: Arc<Led>,
        ring: Ring,
        tuner: Arc<Tuner>,
        buffer: Arc<RingBuffer>,
        settings: Settings,
        nvs: EspDefaultNvsPartition,
    ) -> Result<()> {
        let ui = Self {
            left: Switch::new(pins.left)?,
            right: Switch::new(pins.right)?,
            push: Switch::new(pins.push)?,
            led,
            ring,
            tuner,
            buffer,
            nvs,
            settings,
            mode: Mode::Station,
            pulsing: false,
            epoch: Instant::now(),
            pushed_at: None,
            changed_at: None,
        };
        thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                if let Err(e) = ui.run() {
                    log::error!("input thread: {e:?}");
                }
            })?;
        Ok(())
    }

    fn run(mut self) -> Result<()> {
        self.draw();
        loop {
            FreeRtos::delay_ms(POLL_DELAY_MS);
            if self.left.poll() == Some(true) {
                self.step(Step::Left);
            }
            if self.right.poll() == Some(true) {
                self.step(Step::Right);
            }
            match self.push.poll() {
                Some(true) => self.pushed_at = Some(Instant::now()),
                // only handle the change if we have a `pushed_at` and clear the timer
                Some(false) if self.pushed_at.take().is_some() => {
                    self.mode = self.mode.toggle();
                    log::info!("mode: {:?}", self.mode);
                    self.draw();
                }
                _ => {}
            }
            if self.pushed_at.is_some_and(|t| t.elapsed() >= LONG_PRESS) {
                return self.sleep();
            }
            if self.changed_at.is_some_and(|t| t.elapsed() >= SETTLE) {
                self.changed_at = None;
                self.commit();
            }
            // keep animating while buffering, and draw once more after it ends
            if self.pulsing || self.should_pulse() {
                self.draw();
            }
        }
    }

    /// Move the pending station or the volume by one step.
    fn step(&mut self, step: Step) {
        let s = &mut self.settings;
        match self.mode {
            Mode::Station => {
                let n = STATION_COUNT;
                s.station = match step {
                    Step::Left => s.station.checked_sub(1).unwrap_or(n - 1),
                    Step::Right => (s.station + 1) % n,
                };
                log::info!("station {} (pending)", s.station);
            }
            Mode::Volume => {
                s.volume = match step {
                    Step::Left => s.volume.saturating_sub(1),
                    Step::Right => (s.volume + 1).min(MAX_VOLUME),
                };
                log::info!("volume {}/{MAX_VOLUME}", s.volume);
                // set the DAC volume here once it's wired in, the settle delay is only for saving
            }
        }
        self.changed_at = Some(Instant::now());
        self.draw();
    }

    /// Apply the pending station and save the settings.
    fn commit(&self) {
        if self.tuner.get() != self.settings.station {
            log::info!("tuning to station {}", self.settings.station);
            self.tuner.set(self.settings.station);
        }
        if let Err(e) = self.settings.save(self.nvs.clone()) {
            log::warn!("could not save settings: {e:?}");
        }
    }

    /// Show the current station or volume on the ring.
    fn draw(&mut self) {
        let pulsing = self.should_pulse();
        // start each pulse at full brightness
        if pulsing && !self.pulsing {
            self.epoch = Instant::now();
        }
        self.pulsing = pulsing;
        let res = match self.mode {
            Mode::Station => {
                let color = if self.pulsing {
                    Color::STATION.scale(self.pulse())
                } else {
                    Color::STATION
                };
                self.ring.station(self.settings.station, color)
            }
            Mode::Volume => self.ring.volume(self.settings.volume),
        };
        if let Err(e) = res {
            log::warn!("could not update LED ring: {e:?}");
        }
    }

    fn should_pulse(&self) -> bool {
        matches!(self.mode, Mode::Station) && self.buffer.is_filling()
    }

    /// Current pulse level for [`Color::scale`], a triangle wave from [`PULSE_MIN`] to 255 (the full station color).
    fn pulse(&self) -> u8 {
        let period = PULSE_PERIOD.as_millis();
        let phase = self.epoch.elapsed().as_millis() % period;
        // 0 at the middle of the period, `period` at both ends
        let tri = phase.abs_diff(period / 2) * 2;
        let range = u128::from(u8::MAX - PULSE_MIN);
        u8::try_from(u128::from(PULSE_MIN) + tri * range / period).unwrap_or(u8::MAX)
    }

    /// Turn everything off, wait for the button release and enter deep sleep.
    fn sleep(&mut self) -> Result<()> {
        log::info!("long press, going to sleep");
        // turn the LEDs off to indicate the press was long enough
        self.led.set(Color::OFF)?;
        self.ring.shutdown()?;
        while self.push.pin.is_low() {
            FreeRtos::delay_ms(POLL_DELAY_MS);
        }
        // a bounce at sleep start would wake the device right away
        FreeRtos::delay_ms(DEBOUNCE_TICKS * POLL_DELAY_MS);
        unsafe { esp_wifi_stop() };
        let pin = self.push.pin.pin().into();
        esp!(unsafe { rtc_gpio_pullup_en(pin) })?;
        esp!(unsafe { esp_sleep_enable_ext0_wakeup(pin, 0) })?;
        unsafe { esp_deep_sleep_start() }
    }
}

/// Encoder input mode.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Change the station
    Station,

    /// Adjust the volume.
    Volume,
}

impl Mode {
    fn toggle(self) -> Self {
        match self {
            Self::Station => Self::Volume,
            Self::Volume => Self::Station,
        }
    }
}

/// One encoder detent.
#[derive(Clone, Copy)]
enum Step {
    Left,
    Right,
}

/// A debounced active-low switch, sampled on each poll.
struct Switch {
    pin: PinDriver<'static, Input>,
    pressed: bool,
    /// Number of polls reading the opposite of `pressed`, used for debouncing.
    count: u32,
}

impl Switch {
    fn new(pin: AnyInputPin<'static>) -> Result<Self> {
        let pin = PinDriver::input(pin, Pull::Up)?;
        Ok(Self {
            pressed: pin.is_low(),
            pin,
            count: 0,
        })
    }

    /// Sample the pin.
    ///
    /// Returns the new state once a change exceeds the debounce time.
    fn poll(&mut self) -> Option<bool> {
        if self.pin.is_low() == self.pressed {
            self.count = 0;
            return None;
        }
        self.count = self.count.saturating_add(1);
        if self.count < DEBOUNCE_TICKS {
            return None;
        }
        self.count = 0;
        self.pressed = !self.pressed;
        Some(self.pressed)
    }
}
