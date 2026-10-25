use std::{convert::Infallible, sync::Arc, thread, time::Duration};

use anyhow::{Result, bail};
use esp_idf_svc::{
    http::{
        Method,
        client::{Configuration, EspHttpConnection, FollowRedirectsPolicy},
    },
    nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault},
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

/// Hops after the station URL, NTS uses 2.
const MAX_REDIRECTS: usize = 3;

/// Size of the buffer used to load a cached edge URL from NVS.
const URL_MAX: usize = 256;

/// NVS namespace of the cached edge URLs, keyed by station index.
const NAMESPACE: &str = "edges";

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The network thread, streaming one station into the ring buffer.
pub struct Stream {
    station: usize,
    buffer: Arc<RingBuffer>,
    edges: EspNvs<NvsDefault>,
}

impl Stream {
    /// Start streaming the station at index `station` into `buffer` on a new thread.
    pub fn spawn(
        station: usize,
        buffer: Arc<RingBuffer>,
        nvs: EspDefaultNvsPartition,
    ) -> Result<()> {
        let stream = Self {
            station,
            buffer,
            edges: EspNvs::new(nvs, NAMESPACE, true)?,
        };
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
        let mut conn = self.connect()?;
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

    /// Open the station's audio stream, trying the cached edge URL first.
    ///
    /// Following the redirects from the station URL is slow. We cache the resolved URL to speed up connecting.
    fn connect(&self) -> Result<EspHttpConnection> {
        let key = format!("s{}", self.station);
        let mut buf = [0; URL_MAX];
        if let Ok(Some(edge)) = self.edges.get_str(&key, &mut buf) {
            log::info!("GET {edge} (cached)");
            match Hop::get(edge) {
                Ok(Hop::Stream(conn)) => {
                    log::info!("streaming {edge}");
                    return Ok(conn);
                }
                Ok(Hop::Redirect(next)) => log::warn!("cached edge redirected to {next}"),
                Err(e) => log::warn!("cached edge failed: {e:?}"),
            }
            self.edges.remove(&key).ok();
        }
        let mut url = STATIONS[self.station].to_string();
        for _ in 0..=MAX_REDIRECTS {
            log::info!("GET {url}");
            match Hop::get(&url)? {
                Hop::Redirect(next) => url = next,
                Hop::Stream(conn) => {
                    if let Err(e) = self.edges.set_str(&key, &url) {
                        log::warn!("could not save edge: {e:?}");
                    }
                    log::info!(
                        "streaming {url}, saved as edge for station {}",
                        self.station
                    );
                    return Ok(conn);
                }
            }
        }
        bail!("too many redirects")
    }
}

/// Response for the GET (without automatic redirect following).
enum Hop {
    Redirect(String),
    Stream(EspHttpConnection),
}

impl Hop {
    /// GET `url` and return either the redirect target or the audio stream.
    fn get(url: &str) -> Result<Self> {
        let mut conn = EspHttpConnection::new(&Configuration {
            buffer_size: Some(CHUNK),
            timeout: Some(TIMEOUT),
            follow_redirects_policy: FollowRedirectsPolicy::FollowNone,
            crt_bundle_attach: Some(esp_crt_bundle_attach),
            ..Default::default()
        })?;
        conn.initiate_request(Method::Get, url, &[])?;
        conn.initiate_response()?;
        match conn.status() {
            301 | 302 | 303 | 307 | 308 => {
                let next = conn.header("Location").unwrap_or_default();
                if !next.starts_with("https://") {
                    bail!("unexpected redirect to {next:?}");
                }
                Ok(Self::Redirect(next.to_string()))
            }
            200 => {
                let content_type = conn.header("Content-Type").unwrap_or_default();
                if content_type != "audio/mpeg" {
                    bail!("unexpected content type {content_type:?}");
                }
                Ok(Self::Stream(conn))
            }
            status => bail!("HTTP status {status}"),
        }
    }
}
