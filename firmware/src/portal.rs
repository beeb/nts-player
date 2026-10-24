use std::{
    cmp::Reverse,
    collections::HashSet,
    fmt::Write as _,
    net::{Ipv4Addr, UdpSocket},
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::Result;
use esp_idf_svc::{
    hal::reset::restart,
    http::{
        Method,
        server::{Configuration as HttpConfiguration, EspHttpServer},
    },
    io::{Write, utils::try_read_full},
    ipv4::{self, RouterConfiguration},
    netif::{EspNetif, NetifConfiguration},
    nvs::EspDefaultNvsPartition,
    wifi::{AccessPointConfiguration, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};

use crate::wifi::Credentials;

const AP_SSID: &str = "nts-player";
const FORM: &str = include_str!("portal.html");

/// Run the wifi setup portal, then restart the device.
///
/// The device restarts once credentials are submitted and saved, or after `timeout`.
pub fn run(
    mut wifi: BlockingWifi<EspWifi<'static>>,
    nvs: EspDefaultNvsPartition,
    timeout: Option<Duration>,
) -> Result<()> {
    let ssids = scan(&mut wifi)?;

    let router = RouterConfiguration::default();
    let ip = router.subnet.gateway;
    wifi.wifi_mut()
        .swap_netif_ap(EspNetif::new_with_conf(&NetifConfiguration {
            key: ["WIFI_AP_PORTAL"].into_iter().collect(),
            ip_configuration: Some(ipv4::Configuration::Router(RouterConfiguration {
                dns: Some(ip),
                secondary_dns: None,
                ..router
            })),
            ..NetifConfiguration::wifi_default_router()
        })?)?;
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: [AP_SSID].into_iter().collect(),
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.wait_netif_up()?;
    log::info!("Setup portal running on {AP_SSID}, http://{ip}/");

    thread::Builder::new().stack_size(8 * 1024).spawn(move || {
        if let Err(e) = dns(ip) {
            log::error!("DNS server stopped: {e:?}");
        }
    })?;

    let (tx, rx) = mpsc::channel();
    let page = FORM.replace("{options}", &options(&ssids));
    let location = format!("http://{ip}/");
    let mut server = EspHttpServer::new(&HttpConfiguration {
        uri_match_wildcard: true,
        ..Default::default()
    })?;
    server.fn_handler("/", Method::Get, move |req| {
        req.into_ok_response()?.write_all(page.as_bytes())
    })?;
    server.fn_handler::<anyhow::Error, _>("/", Method::Post, move |mut req| {
        // fits the longest valid input with every byte percent-encoded
        let mut body = [0; 512];
        let len = try_read_full(&mut req, &mut body).map_err(|(e, _)| e)?;
        let Some(creds) = Credentials::from_form(&body[..len]) else {
            req.into_status_response(400)?
                .write_all(b"Invalid network or password")?;
            return Ok(());
        };
        creds.save(nvs.clone())?;
        req.into_ok_response()?.write_all(b"Saved, restarting...")?;
        tx.send(())?;
        Ok(())
    })?;
    // catches the OS connectivity probes, which makes phones open the portal
    server.fn_handler("/*", Method::Get, move |req| {
        req.into_response(302, None, &[("Location", location.as_str())])
            .map(|_| ())
    })?;

    if let Some(timeout) = timeout {
        // if the user didn't submit the form after the timeout, we restart to
        // retry the saved credentials
        rx.recv_timeout(timeout).ok();
    } else {
        rx.recv()?;
    }
    // let the response reach the client
    thread::sleep(Duration::from_secs(1));
    restart();
}

/// Scan for nearby networks and return their SSIDs sorted by descending signal
/// strength.
fn scan(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<Vec<String>> {
    if wifi.is_started()? {
        wifi.stop()?;
    }
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;
    let mut aps = wifi.scan()?;
    wifi.stop()?;
    aps.sort_by_key(|ap| Reverse(ap.signal_strength));
    let mut seen = HashSet::new();
    Ok(aps
        .iter()
        .map(|ap| ap.ssid.as_str())
        .filter(|s| !s.is_empty() && seen.insert(*s))
        .map(str::to_owned)
        .collect())
}

/// Render the SSID `<option>` elements for the dropdown.
fn options(ssids: &[String]) -> String {
    if ssids.is_empty() {
        return r#"<option value="" disabled selected>No networks found</option>"#.to_owned();
    }
    ssids.iter().fold(String::new(), |mut out, s| {
        let s = escape(s);
        let _ = write!(out, r#"<option value="{s}">{s}</option>"#);
        out
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Answer every DNS query with `ip` to make the portal open.
fn dns(ip: Ipv4Addr) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:53")?;
    let mut request = [0; 512];
    let mut response = [0; 512];
    loop {
        let (len, src) = socket.recv_from(&mut request)?;
        // malformed requests are skipped
        if let Ok(len) = edge_captive::reply(
            &request[..len],
            &ip.octets(),
            Duration::from_mins(1),
            &mut response,
        ) {
            socket.send_to(&response[..len], src)?;
        }
    }
}
