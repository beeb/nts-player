use anyhow::{Result, anyhow};
use esp_idf_svc::{
    nvs::{EspDefaultNvsPartition, EspNvs},
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use heapless::String;

const NAMESPACE: &str = "wifi";

/// Credentials of the wifi network saved in NVS.
pub struct Credentials {
    pub ssid: String<32>,
    pub password: String<64>,
}

impl Credentials {
    /// Parse the credentials from the portal's url-encoded form body.
    ///
    /// Returns `None` if the SSID is missing or a field is too long for the wifi driver.
    pub fn from_form(body: &[u8]) -> Option<Self> {
        let mut ssid = None;
        let mut password = Some(String::new());
        for (key, value) in form_urlencoded::parse(body) {
            match key.as_ref() {
                "ssid" => ssid = value.as_ref().try_into().ok(),
                "password" => password = value.as_ref().try_into().ok(),
                _ => {}
            }
        }
        let ssid = ssid.filter(|s: &String<32>| !s.is_empty())?;
        Some(Self {
            ssid,
            password: password?,
        })
    }

    /// Load the credentials from NVS, if any.
    pub fn load(nvs: EspDefaultNvsPartition) -> Result<Option<Self>> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        // enough for a 64-byte password and the nul terminator
        let mut buf = [0; 65];
        let Some(ssid) = nvs.get_str("ssid", &mut buf)? else {
            return Ok(None);
        };
        let ssid = ssid
            .try_into()
            .map_err(|_| anyhow!("saved SSID too long"))?;
        let password = nvs
            .get_str("password", &mut buf)?
            .unwrap_or_default()
            .try_into()
            .map_err(|_| anyhow!("saved password too long"))?;
        Ok(Some(Self { ssid, password }))
    }

    pub fn save(&self, nvs: EspDefaultNvsPartition) -> Result<()> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        nvs.set_str("ssid", &self.ssid)?;
        nvs.set_str("password", &self.password)?;
        Ok(())
    }

    /// Connect to the network in station mode and wait for an IP address.
    pub fn connect(&self, wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
        // make sure to stop wifi in case a previous attempt left it enabled
        if wifi.is_started()? {
            wifi.stop()?;
        }
        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: self.ssid.clone(),
            password: self.password.clone(),
            auth_method: if self.password.is_empty() {
                AuthMethod::None
            } else {
                AuthMethod::WPA2Personal
            },
            ..Default::default()
        }))?;
        wifi.start()?;
        wifi.connect()?;
        wifi.wait_netif_up()?;
        Ok(())
    }
}
