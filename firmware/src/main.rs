use std::{sync::Arc, thread, time::Duration};

use anyhow::Result;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{peripherals::Peripherals, reset::restart},
    nvs::EspDefaultNvsPartition,
    wifi::{BlockingWifi, EspWifi},
};

use crate::{
    audio::{I2sPins, Player},
    buffer::RingBuffer,
    input::Ui,
    led::{Color, Led},
    stream::Stream,
    wifi::Credentials,
};

mod audio;
mod buffer;
mod input;
mod led;
mod portal;
mod prelude;
mod stream;
mod wifi;

const CONNECT_ATTEMPTS: usize = 3;
const PORTAL_TIMEOUT: Duration = Duration::from_mins(5);

/// About 5 s of audio at 256 kbps.
const BUFFER_SIZE: usize = 160 * 1024;

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut led = None;
    if let Err(e) = run(&mut led) {
        log::error!("{e:?}");
        if let Some(led) = &led {
            led.set(Color::DBG_RED).ok();
        }
        thread::sleep(Duration::from_secs(5));
        restart();
    }
}

/// Run the app, storing the LED in `led` so that `main` can still use it on error.
fn run(led: &mut Option<Arc<Led>>) -> Result<()> {
    let peripherals = Peripherals::take()?;
    let led = led.insert(Arc::new(Led::new(peripherals.pins.gpio38)?));
    Ui::spawn(peripherals.pins.gpio0, led.clone())?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs.clone()))?,
        sysloop,
    )?;

    let Some(mut creds) = Credentials::load(nvs.clone())? else {
        log::info!("no wifi credentials saved");
        led.set(Color::DBG_BLUE)?;
        return portal::run(wifi, nvs, None); // stay on portal forever
    };
    led.set(Color::DBG_YELLOW)?;
    let connected = (1..=CONNECT_ATTEMPTS).any(|attempt| {
        creds
            .connect(&mut wifi)
            .inspect_err(|e| {
                log::warn!(
                    "connection attempt {attempt} to {} failed: {e:?}",
                    creds.ssid
                );
            })
            .is_ok()
    });
    if !connected {
        // connecting with saved credentials failed, let's show the portal
        // again for a little bit before we retry
        led.set(Color::DBG_BLUE)?;
        return portal::run(wifi, nvs, Some(PORTAL_TIMEOUT));
    }
    led.set(Color::DBG_GREEN)?;
    log::info!(
        "connected to {}, ip: {}",
        creds.ssid,
        wifi.wifi().sta_netif().get_ip_info()?.ip
    );
    if let Err(e) = creds.save_channel(nvs.clone(), &wifi) {
        log::warn!("could not save wifi channel: {e:?}");
    }

    let buffer = Arc::new(RingBuffer::new(BUFFER_SIZE));
    Stream::spawn(0, buffer.clone(), nvs)?;
    Player::spawn(
        buffer,
        I2sPins {
            i2s: peripherals.i2s0,
            bck: peripherals.pins.gpio5.degrade_input_output(),
            din: peripherals.pins.gpio6.degrade_output(),
            lrck: peripherals.pins.gpio7.degrade_input_output(),
        },
    )?;
    // keep `wifi` alive
    loop {
        thread::park();
    }
}
