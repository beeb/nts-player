use std::{thread, time::Duration};

use anyhow::Result;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{peripherals::Peripherals, reset::restart},
    nvs::EspDefaultNvsPartition,
    wifi::{BlockingWifi, EspWifi},
};

use crate::wifi::Credentials;

mod portal;
mod wifi;

const CONNECT_ATTEMPTS: usize = 3;
const PORTAL_TIMEOUT: Duration = Duration::from_mins(5);

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    if let Err(e) = run() {
        log::error!("{e:?}");
        thread::sleep(Duration::from_secs(5));
        restart();
    }
}

fn run() -> Result<()> {
    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs.clone()))?,
        sysloop,
    )?;

    let Some(creds) = Credentials::load(nvs.clone())? else {
        log::info!("no wifi credentials saved");
        return portal::run(wifi, nvs, None); // stay on portal forever
    };
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
        return portal::run(wifi, nvs, Some(PORTAL_TIMEOUT));
    }
    log::info!(
        "connected to {}, ip: {}",
        creds.ssid,
        wifi.wifi().sta_netif().get_ip_info()?.ip
    );

    // keep the connection alive
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
