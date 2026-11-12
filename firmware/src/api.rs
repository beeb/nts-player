//! A minimal `ESPHome` native API server for the Home Assistant integration.

use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc::Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use esp_idf_svc::{mdns::EspMdns, netif::EspNetif};
use prost::{Message, encoding::encode_varint};

use crate::{
    prelude::OrPanic,
    settings::{Settings, Volume},
    stream::{STATION_COUNT, STATIONS},
};

mod proto;

use proto::{
    AUTHENTICATION_REQUEST, AuthenticationResponse, BUTTON_COMMAND_REQUEST, ButtonCommandRequest,
    DEVICE_CAPABILITIES_REQUEST, DEVICE_INFO_REQUEST, DISCONNECT_REQUEST,
    DeviceCapabilitiesResponse, DeviceInfoResponse, DisconnectResponse, HELLO_REQUEST,
    HelloResponse, LIST_ENTITIES_REQUEST, ListEntitiesButtonResponse, ListEntitiesDoneResponse,
    ListEntitiesNumberResponse, ListEntitiesSelectResponse, ListEntitiesSwitchResponse,
    NUMBER_COMMAND_REQUEST, NUMBER_MODE_SLIDER, NumberCommandRequest, NumberStateResponse,
    PING_REQUEST, PingResponse, SELECT_COMMAND_REQUEST, SUBSCRIBE_STATES_REQUEST,
    SWITCH_COMMAND_REQUEST, SelectCommandRequest, SelectStateResponse, SwitchCommandRequest,
    SwitchStateResponse, Typed,
};

/// Device name, also the mDNS host name. HA requires both to match.
pub const NAME: &str = "nts-player";

pub const FRIENDLY_NAME: &str = "NTS Player";

/// The `ESPHome` API port.
pub const PORT: u16 = 6053;

const STACK_SIZE: usize = 8 * 1024;

/// Board name in the mDNS TXT records.
const BOARD: &str = "esp32-s3-devkitc-1";

/// Read timeout of the client socket, which is also the period of the state checks.
const TICK: Duration = Duration::from_millis(100);

/// Timeout for clients, they get dropped if they don't reply quickly enough.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// Largest accepted payload, larger frames drop the client.
const MAX_PAYLOAD: usize = 1024;

const POWER_KEY: u32 = 1;
const STATION_KEY: u32 = 2;
const VOLUME_KEY: u32 = 3;
const PREV_STATION_KEY: u32 = 4;
const NEXT_STATION_KEY: u32 = 5;

/// The state shown in HA.
#[derive(Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// Whether the device plays, `false` in standby.
    pub power: bool,
    pub station: u8,
    pub volume: Volume,
}

impl Snapshot {
    pub fn new(settings: Settings) -> Self {
        Self {
            power: true,
            station: settings.station,
            volume: settings.volume,
        }
    }
}

/// A command for the UI thread, from HA or the update server.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Set playing or standby.
    Power(bool),

    /// The station index, must be in [`STATIONS`].
    Station(u8),

    /// The volume level
    Volume(Volume),

    /// Firmware update is running, light up a number of LEDs, up to [`RING_LEDS`](crate::led::RING_LEDS).
    Update(u8),

    /// The firmware update stopped, the old firmware stays active.
    UpdateFailed,
}

/// The API server thread.
pub struct Api {
    listener: TcpListener,
    snapshot: Arc<Mutex<Snapshot>>,
    /// MAC address of the wifi station interface.
    mac: [u8; 6],
    commands: Sender<Command>,
    /// The mDNS announcement, `None` if it failed to start.
    _mdns: Option<EspMdns>,
}

impl Api {
    /// Announce the server on mDNS and start it on a new thread.
    pub fn spawn(
        netif: &EspNetif,
        snapshot: Arc<Mutex<Snapshot>>,
        commands: Sender<Command>,
    ) -> Result<()> {
        let mac = netif.get_mac()?;
        let listener = TcpListener::bind(("0.0.0.0", PORT))?;
        listener.set_nonblocking(true)?;
        let api = Self {
            listener,
            snapshot,
            mac,
            commands,
            // without mDNS, HA can still add the device by IP
            _mdns: announce(mac)
                .inspect_err(|e| log::warn!("mDNS failed: {e:?}"))
                .ok(),
        };
        thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || api.run())?;
        log::info!("API listening on port {PORT}");
        Ok(())
    }

    /// Serve one client at a time, forever.
    ///
    /// A new client replaces the current one.
    fn run(self) -> ! {
        let mut client = None;
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    log::info!("API client {peer} connected");
                    client = Client::new(stream)
                        .inspect_err(|e| log::warn!("could not set up the API client: {e:?}"))
                        .ok();
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => log::warn!("API accept failed: {e:?}"),
            }
            let Some(c) = &mut client else {
                thread::sleep(TICK);
                continue;
            };
            match self.serve(c) {
                Ok(true) => {}
                Ok(false) => {
                    log::info!("API client disconnected");
                    client = None;
                }
                Err(e) => {
                    log::warn!("API client dropped: {e:?}");
                    client = None;
                }
            }
        }
    }

    /// Read from the client for up to one [`TICK`], handle the complete frames and send the state changes.
    ///
    /// Returns `false` once the client is gone.
    fn serve(&self, c: &mut Client) -> Result<bool> {
        let mut chunk = [0; 256];
        match c.stream.read(&mut chunk) {
            Ok(0) => return Ok(false),
            Ok(n) => c
                .buf
                .extend_from_slice(chunk.get(..n).context("read past the chunk")?),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.into()),
        }
        while let Some(f) = Frame::parse(&c.buf)? {
            let payload: Vec<u8> = c.buf.drain(..f.end).skip(f.start).collect();
            if !self.handle(c, f.ty, &payload)? {
                return Ok(false);
            }
        }
        self.push_states(c)?;
        Ok(true)
    }

    /// Handle one request. Returns `false` if the client asked to disconnect.
    fn handle(&self, c: &mut Client, ty: u32, payload: &[u8]) -> Result<bool> {
        match ty {
            HELLO_REQUEST => c.send(&HelloResponse {
                api_version_major: proto::API_VERSION_MAJOR,
                api_version_minor: proto::API_VERSION_MINOR,
                server_info: format!("{NAME} {}", env!("CARGO_PKG_VERSION")),
                name: NAME.to_string(),
            })?,
            AUTHENTICATION_REQUEST => c.send(&AuthenticationResponse {
                invalid_password: false,
            })?,
            DISCONNECT_REQUEST => {
                c.send(&DisconnectResponse {})?;
                return Ok(false);
            }
            PING_REQUEST => c.send(&PingResponse {})?,
            DEVICE_INFO_REQUEST => c.send(&self.device_info())?,
            DEVICE_CAPABILITIES_REQUEST => c.send(&DeviceCapabilitiesResponse {})?,
            LIST_ENTITIES_REQUEST => list_entities(c)?,
            SUBSCRIBE_STATES_REQUEST => {
                c.subscribed = true;
                // send every state on the next push
                c.sent = None;
            }
            SWITCH_COMMAND_REQUEST => {
                let m = SwitchCommandRequest::decode(payload)?;
                if m.key == POWER_KEY {
                    self.commands.send(Command::Power(m.state))?;
                }
            }
            SELECT_COMMAND_REQUEST => {
                let m = SelectCommandRequest::decode(payload)?;
                let station = STATIONS.iter().position(|s| s.name == m.state);
                if m.key == STATION_KEY
                    && let Some(i) = station
                {
                    self.commands.send(Command::Station(u8::try_from(i)?))?;
                } else {
                    log::warn!("ignoring select command {m:?}");
                }
            }
            NUMBER_COMMAND_REQUEST => {
                let m = NumberCommandRequest::decode(payload)?;
                if m.key == VOLUME_KEY && m.state.is_finite() {
                    self.commands
                        .send(Command::Volume(Volume::from_f32(m.state)))?;
                } else {
                    log::warn!("ignoring number command {m:?}");
                }
            }
            BUTTON_COMMAND_REQUEST => {
                let m = ButtonCommandRequest::decode(payload)?;
                let station = self
                    .snapshot
                    .lock()
                    .or_panic("snapshot mutex poisoned")
                    .station;
                let n = STATION_COUNT;
                let station = match m.key {
                    PREV_STATION_KEY => station.checked_sub(1).unwrap_or(n - 1),
                    NEXT_STATION_KEY => (station + 1) % n,
                    _ => {
                        log::warn!("ignoring button command {m:?}");
                        return Ok(true);
                    }
                };
                self.commands.send(Command::Station(station))?;
            }
            // HA subscribes to logs, services, HA states and more, which the device doesn't offer
            _ => log::debug!("skipping API message {ty}"),
        }
        Ok(true)
    }

    fn device_info(&self) -> DeviceInfoResponse {
        DeviceInfoResponse {
            name: NAME.to_string(),
            mac_address: self.mac.map(|b| format!("{b:02X}")).join(":"),
            esphome_version: proto::ESPHOME_VERSION.to_string(),
            model: "ESP32-S3 web radio".to_string(),
            manufacturer: "DIY".to_string(),
            friendly_name: FRIENDLY_NAME.to_string(),
        }
    }

    /// Send the states that changed since the last push, or all of them after a subscribe.
    fn push_states(&self, c: &mut Client) -> Result<()> {
        if !c.subscribed {
            return Ok(());
        }
        let now = *self.snapshot.lock().or_panic("snapshot mutex poisoned");
        let old = c.sent;
        if old.is_none_or(|o| o.power != now.power) {
            c.send(&SwitchStateResponse {
                key: POWER_KEY,
                state: now.power,
            })?;
        }
        if old.is_none_or(|o| o.station != now.station) {
            c.send(&SelectStateResponse {
                key: STATION_KEY,
                state: STATIONS
                    .get(usize::from(now.station))
                    .map(|s| s.name.to_string())
                    .unwrap_or_default(),
            })?;
        }
        if old.is_none_or(|o| o.volume != now.volume) {
            c.send(&NumberStateResponse {
                key: VOLUME_KEY,
                state: f32::from(now.volume.get()),
            })?;
        }
        c.sent = Some(now);
        Ok(())
    }
}

/// One connected API client.
struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    /// Whether the client subscribed to state changes.
    subscribed: bool,
    /// The last state sent, or `None` before the first push.
    sent: Option<Snapshot>,
}

impl Client {
    fn new(stream: TcpStream) -> Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(TICK))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            buf: Vec::new(),
            subscribed: false,
            sent: None,
        })
    }

    /// Send `msg` in a plaintext frame.
    fn send<T: Typed>(&mut self, msg: &T) -> Result<()> {
        let body = msg.encode_to_vec();
        let mut frame = vec![0];
        encode_varint(body.len() as u64, &mut frame);
        encode_varint(u64::from(T::ID), &mut frame);
        frame.extend(body);
        self.stream.write_all(&frame)?;
        Ok(())
    }
}

/// Position of one complete plaintext frame at the start of a buffer.
struct Frame {
    /// The message id.
    ty: u32,
    /// Start of the payload.
    start: usize,
    /// End of the payload and of the frame.
    end: usize,
}

impl Frame {
    /// Parse the frame at the start of `buf`: `0x00`, varint payload length, varint message id, payload.
    ///
    /// Returns `None` until `buf` holds the whole frame.
    fn parse(buf: &[u8]) -> Result<Option<Self>> {
        let Some(&first) = buf.first() else {
            return Ok(None);
        };
        if first != 0 {
            bail!("not a plaintext frame, first byte {first:#04x}");
        }
        let rest = buf.get(1..).unwrap_or_default();
        let Some((len, a)) = varint(rest)? else {
            return Ok(None);
        };
        let Some((ty, b)) = varint(rest.get(a..).unwrap_or_default())? else {
            return Ok(None);
        };
        let len = usize::try_from(len)?;
        if len > MAX_PAYLOAD {
            bail!("frame too large: {len} bytes");
        }
        let start = 1 + a + b;
        let end = start + len;
        if buf.len() < end {
            return Ok(None);
        }
        Ok(Some(Self {
            ty: u32::try_from(ty)?,
            start,
            end,
        }))
    }
}

/// Announce the API on mDNS like an `ESPHome` device (for HA discovery).
///
/// The returned handle must stay alive.
fn announce(mac: [u8; 6]) -> Result<EspMdns> {
    let mut mdns = EspMdns::take()?;
    mdns.set_hostname(NAME)?;
    mdns.set_instance_name(FRIENDLY_NAME)?;
    let mac = mac.map(|b| format!("{b:02x}")).join("");
    mdns.add_service(
        Some(NAME),
        "_esphomelib",
        "_tcp",
        PORT,
        &[
            ("version", proto::ESPHOME_VERSION),
            ("mac", mac.as_str()),
            ("platform", "ESP32"),
            ("board", BOARD),
            ("network", "wifi"),
            ("friendly_name", FRIENDLY_NAME),
        ],
    )?;
    log::info!("mDNS: {NAME}.local, _esphomelib._tcp port {PORT}");
    Ok(mdns)
}

/// Send the entity descriptions, then the end marker.
fn list_entities(c: &mut Client) -> Result<()> {
    c.send(&ListEntitiesSwitchResponse {
        object_id: "power".to_string(),
        key: POWER_KEY,
        name: "Power".to_string(),
    })?;
    c.send(&ListEntitiesSelectResponse {
        object_id: "station".to_string(),
        key: STATION_KEY,
        name: "Station".to_string(),
        options: STATIONS.iter().map(|s| s.name.to_string()).collect(),
    })?;
    c.send(&ListEntitiesNumberResponse {
        object_id: "volume".to_string(),
        key: VOLUME_KEY,
        name: "Volume".to_string(),
        min_value: 0.0,
        max_value: f32::from(Volume::MAX.get()),
        step: 1.0,
        mode: NUMBER_MODE_SLIDER,
    })?;
    c.send(&ListEntitiesButtonResponse {
        object_id: "previous_station".to_string(),
        key: PREV_STATION_KEY,
        name: "Previous station".to_string(),
        icon: "mdi:skip-previous".to_string(),
    })?;
    c.send(&ListEntitiesButtonResponse {
        object_id: "next_station".to_string(),
        key: NEXT_STATION_KEY,
        name: "Next station".to_string(),
        icon: "mdi:skip-next".to_string(),
    })?;
    c.send(&ListEntitiesDoneResponse {})
}

/// Read a protobuf varint at the start of `buf`.
///
/// Returns the value and its length in bytes, or `None` if `buf` ends before.
fn varint(buf: &[u8]) -> Result<Option<(u64, usize)>> {
    let mut value = 0;
    for (i, &b) in buf.iter().enumerate().take(10) {
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(Some((value, i + 1)));
        }
    }
    if buf.len() >= 10 {
        bail!("varint too long");
    }
    Ok(None)
}
