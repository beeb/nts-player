use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use esp_idf_svc::{
    hal::{
        delay::FreeRtos,
        gpio::{Input, InputPin, PinDriver, Pull},
    },
    sys::{
        esp, esp_deep_sleep_start, esp_sleep_enable_ext0_wakeup, esp_wifi_stop, rtc_gpio_pullup_en,
    },
};

use crate::led::{Color, Led};

/// Hold time for putting the device to sleep.
const LONG_PRESS: Duration = Duration::from_millis(700);

/// Polling interval for the button, in milliseconds.
const POLL_DELAY_MS: u32 = 10;

/// Button debounce time, in milliseconds.
const DEBOUNCE_MS: u32 = 30;

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

/// The input and UI thread state.
pub struct Ui {
    button: PinDriver<'static, Input>,
    led: Arc<Led>,
    mode: Mode,
}

impl Ui {
    /// Start the input thread with an active-low push button on `pin`.
    pub fn spawn(pin: impl InputPin + 'static, led: Arc<Led>) -> Result<()> {
        let ui = Self {
            button: PinDriver::input(pin, Pull::Up)?,
            led,
            mode: Mode::Station,
        };
        thread::spawn(move || {
            if let Err(e) = ui.run() {
                log::error!("input thread: {e:?}");
            }
        });
        Ok(())
    }

    fn run(mut self) -> Result<()> {
        // ignore the first press (it woke the device)
        self.wait_release();
        loop {
            while self.button.is_high() {
                FreeRtos::delay_ms(POLL_DELAY_MS);
            }
            FreeRtos::delay_ms(DEBOUNCE_MS);
            let start = Instant::now();
            while self.button.is_low() {
                if start.elapsed() >= LONG_PRESS {
                    return self.sleep();
                }
                FreeRtos::delay_ms(POLL_DELAY_MS);
            }
            FreeRtos::delay_ms(DEBOUNCE_MS);
            self.mode = self.mode.toggle();
            log::info!("mode: {:?}", self.mode);
        }
    }

    fn wait_release(&self) {
        while self.button.is_low() {
            FreeRtos::delay_ms(POLL_DELAY_MS);
        }
        FreeRtos::delay_ms(DEBOUNCE_MS);
    }

    /// Turn everything off, wait for the button release and enter deep sleep.
    fn sleep(&self) -> Result<()> {
        log::info!("long press, going to sleep");
        // turn the LED(s) off to indicate the press was long enough
        self.led.set(Color::OFF)?;
        self.wait_release();
        unsafe { esp_wifi_stop() };
        let pin = self.button.pin().into();
        esp!(unsafe { rtc_gpio_pullup_en(pin) })?;
        esp!(unsafe { esp_sleep_enable_ext0_wakeup(pin, 0) })?;
        unsafe { esp_deep_sleep_start() }
    }
}
