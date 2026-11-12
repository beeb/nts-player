//! Firmware updates over Wi-Fi, uploaded from a page served by the device.

use std::{ffi::CStr, sync::mpsc::Sender, thread, time::Duration};

use anyhow::{Context, Result, ensure};
use esp_idf_svc::{
    hal::reset::restart,
    http::{
        Method,
        server::{Configuration, EspHttpConnection, EspHttpServer, Request},
    },
    io::{Write, utils::try_read_full},
    ota::{EspFirmwareInfoLoad, EspOta},
    sys::{ESP_APP_DESC_MAGIC_WORD, ESP_IMAGE_HEADER_MAGIC, esp_chip_id_t_ESP_CHIP_ID_ESP32S3},
};

use crate::{api::Command, led::RING_LEDS};

const PAGE: &str = include_str!("ota.html");

/// Size of the `ota_0` and `ota_1` slots in `partitions.csv`.
const SLOT_SIZE: usize = 0x40_0000;

/// Size of the chunks read from the upload and written to flash.
const CHUNK: usize = 4096;

/// Stack size of the HTTP server task, the image check at the end of the upload needs more than the default.
const STACK_SIZE: usize = 10 * 1024;

/// Time for the response to reach the browser before the restart.
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// The firmware upload server on port 80, it stops when dropped.
pub struct Ota {
    _server: EspHttpServer<'static>,
}

impl Ota {
    /// Start the server and confirm the running firmware.
    ///
    /// The server sends the update progress through `commands`. A new firmware that can't start the server is not
    /// confirmed, so the bootloader rolls it back on the next reset.
    pub fn start(commands: Sender<Command>) -> Result<Self> {
        let mut server = EspHttpServer::new(&Configuration {
            stack_size: STACK_SIZE,
            ..Default::default()
        })?;
        let page = PAGE.replace("{version}", env!("CARGO_PKG_VERSION"));
        server.fn_handler("/", Method::Get, move |req| {
            req.into_ok_response()?.write_all(page.as_bytes())
        })?;
        server.fn_handler::<anyhow::Error, _>("/update", Method::Post, move |mut req| {
            if req.header("Content-Type") != Some("application/octet-stream") {
                log::warn!("firmware upload rejected: wrong content type");
                req.into_status_response(400)?
                    .write_all(b"Upload the file from the update page.")?;
                return Ok(());
            }
            let mut buf = vec![0u8; CHUNK];
            let len = try_read_full(&mut req, &mut buf).map_err(|(e, _)| e)?;
            let head = buf.get(..len).context("read past the buffer")?;
            let size = req.header("Content-Length").and_then(|v| v.parse().ok());
            let size = match check(head, size) {
                Ok(size) => size,
                Err(msg) => {
                    log::warn!("firmware upload rejected: {msg}");
                    req.into_status_response(400)?.write_all(msg.as_bytes())?;
                    return Ok(());
                }
            };
            if let Err(e) = flash(&mut req, &mut buf, len, size, &commands) {
                log::warn!("firmware update failed: {e:?}");
                // the UI restarts into the old firmware
                commands.send(Command::UpdateFailed).ok();
                req.into_status_response(500)?
                    .write_all(format!("Update failed: {e}").as_bytes())?;
                return Ok(());
            }
            log::info!("firmware update done, restarting");
            // restart even if the browser is gone and the reply fails
            thread::spawn(|| {
                thread::sleep(RESTART_DELAY);
                restart();
            });
            req.into_ok_response()?
                .write_all(b"Update done, the player restarts.")?;
            Ok(())
        })?;
        log::info!("firmware update page on port 80");
        if let Err(e) = EspOta::new().and_then(|mut ota| ota.mark_running_slot_valid()) {
            log::warn!("could not confirm the firmware: {e:?}");
        }
        Ok(Self { _server: server })
    }
}

/// Check the firmware file.
///
/// Verify that the size is not too large, and that the firmware is for this chip and project.
///
/// Returns the size, or the message for the browser.
fn check(head: &[u8], size: Option<usize>) -> Result<usize, &'static str> {
    let size = size.ok_or("The upload has no Content-Length.")?;
    if size > SLOT_SIZE {
        return Err("This file is too large to be a firmware.");
    }
    let info = EspFirmwareInfoLoad
        .fetch_native(head)
        .ok_or("This file is too small to be a firmware.")?;
    // copy out of the packed struct, comparing would take a reference
    let magic = info.image_header.magic;
    let chip = info.image_header.chip_id;
    if u32::from(magic) != ESP_IMAGE_HEADER_MAGIC || chip != esp_chip_id_t_ESP_CHIP_ID_ESP32S3 {
        return Err("This file is not an ESP32-S3 firmware.");
    }
    let desc = info.app_desc;
    let name =
        CStr::from_bytes_until_nul(bytemuck::cast_slice::<_, u8>(desc.project_name.as_slice()))
            .ok();
    if desc.magic_word != ESP_APP_DESC_MAGIC_WORD || name != Some(c"nts-player") {
        return Err("This file is not an NTS Player firmware.");
    }
    Ok(size)
}

/// Write an upload of `size` bytes to the free slot and make it the boot slot.
///
/// `buf` holds the first `len` bytes. Sends the progress to the UI, which stops playback on the first message.
fn flash(
    req: &mut Request<&mut EspHttpConnection<'_>>,
    buf: &mut [u8],
    mut len: usize,
    size: usize,
    commands: &Sender<Command>,
) -> Result<()> {
    // the erase in `initiate_update` takes seconds, stop playback first
    commands.send(Command::Update(0))?;
    let mut lit = 0;
    let mut ota = EspOta::new()?;
    let mut update = ota.initiate_update_with_known_size(size)?;
    let mut written = 0;
    while len > 0 {
        update.write(buf.get(..len).context("read past the buffer")?)?;
        written += len;
        let now = u8::try_from(written * usize::from(RING_LEDS) / size)
            .unwrap_or(RING_LEDS)
            .min(RING_LEDS);
        if now != lit {
            lit = now;
            commands.send(Command::Update(lit))?;
        }
        len = try_read_full(&mut *req, buf).map_err(|(e, _)| e)?;
    }
    ensure!(written == size, "got {written} of {size} bytes");
    // check the image hash before switching the boot slot
    update.complete()?;
    Ok(())
}
