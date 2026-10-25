use std::{convert::Infallible, sync::Arc, thread, time::Duration};

use anyhow::{Result, bail};
use esp_idf_svc::{
    http::{
        Method,
        client::{Configuration, EspHttpConnection},
    },
    sys::esp_crt_bundle_attach,
};

use crate::buffer::RingBuffer;

/// Stream URLs
pub const STATIONS: [&str; 18] = [
    "https://stream-relay-geo.ntslive.net/stream",  // NTS 1
    "https://stream-relay-geo.ntslive.net/stream2", // NTS 2
    "https://stream-mixtape-geo.ntslive.net/mixtape4", // Poolside
    "https://stream-mixtape-geo.ntslive.net/mixtape", // Slow Focus
    "https://stream-mixtape-geo.ntslive.net/mixtape2", // Low Key
    "https://stream-mixtape-geo.ntslive.net/mixtape6", // Memory Lane
    "https://stream-mixtape-geo.ntslive.net/mixtape5", // 4 To The Floor
    "https://stream-mixtape-geo.ntslive.net/mixtape21", // Island Time
    "https://stream-mixtape-geo.ntslive.net/mixtape26", // The Tube
    "https://stream-mixtape-geo.ntslive.net/mixtape35", // Sheet Music
    "https://stream-mixtape-geo.ntslive.net/mixtape27", // Feelings
    "https://stream-mixtape-geo.ntslive.net/mixtape3", // Expansions
    "https://stream-mixtape-geo.ntslive.net/mixtape22", // Rap House
    "https://stream-mixtape-geo.ntslive.net/mixtape31", // Labyrinth
    "https://stream-mixtape-geo.ntslive.net/mixtape24", // Sweat
    "https://stream-mixtape-geo.ntslive.net/mixtape36", // Otaku
    "https://stream-mixtape-geo.ntslive.net/mixtape34", // The Pit
    "https://stream-mixtape-geo.ntslive.net/mixtape23", // Field Recordings
];

/// Stack size of the network thread, TLS needs a lot.
const STACK_SIZE: usize = 12 * 1024;

/// Size of the chunks copied from the HTTP body to the ring buffer.
const CHUNK: usize = 4096;

/// A read that takes longer than this fails and triggers a reconnect.
const TIMEOUT: Duration = Duration::from_secs(10);

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The network thread, streaming one station into the ring buffer.
pub struct Stream {
    url: &'static str,
    buffer: Arc<RingBuffer>,
}

impl Stream {
    /// Start streaming `url` into `buffer` on a new thread.
    pub fn spawn(url: &'static str, buffer: Arc<RingBuffer>) -> Result<()> {
        let stream = Self { url, buffer };
        thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || stream.run())?;
        Ok(())
    }

    /// Stream forever, reconnecting with backoff on any error or end of stream.
    fn run(&self) -> ! {
        let mut backoff = MIN_BACKOFF;
        loop {
            let Err(e) = self.stream(&mut backoff);
            log::warn!("stream stopped: {e:?}, retrying in {backoff:?}");
            thread::sleep(backoff);
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// Connect and copy the body into the buffer until something fails.
    fn stream(&self, backoff: &mut Duration) -> Result<Infallible> {
        let mut conn = EspHttpConnection::new(&Configuration {
            buffer_size: Some(CHUNK),
            timeout: Some(TIMEOUT),
            crt_bundle_attach: Some(esp_crt_bundle_attach),
            ..Default::default()
        })?;
        // the default redirect policy follows redirects for GET
        conn.initiate_request(Method::Get, self.url, &[])?;
        conn.initiate_response()?;
        let status = conn.status();
        if status != 200 {
            bail!("HTTP status {status}");
        }
        let content_type = conn.header("Content-Type").unwrap_or_default();
        if content_type != "audio/mpeg" {
            bail!("unexpected content type {content_type:?}");
        }
        log::info!("streaming {}", self.url);
        let mut chunk = vec![0; CHUNK];
        loop {
            let n = conn.read(&mut chunk)?;
            if n == 0 {
                bail!("server closed the stream");
            }
            self.buffer.write(&chunk[..n]);
            *backoff = MIN_BACKOFF;
        }
    }
}
