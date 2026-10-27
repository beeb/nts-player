use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};

use crate::stream::STATION_COUNT;

/// Highest volume step.
pub const MAX_VOLUME: u8 = 18;

const NAMESPACE: &str = "settings";

/// User settings saved in NVS.
#[derive(Clone, Copy)]
pub struct Settings {
    /// Index into [`STATIONS`](crate::stream::STATIONS).
    pub station: u8,

    /// Volume step, from 0 (all LEDs off) to [`MAX_VOLUME`] (all LEDs lit).
    pub volume: u8,
}

impl Settings {
    /// Load the settings from NVS, using defaults for missing or out-of-range values.
    pub fn load(nvs: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        let station = nvs
            .get_u8("station")?
            .filter(|&s| s < STATION_COUNT)
            .unwrap_or(0);
        let volume = nvs
            .get_u8("volume")?
            .filter(|&v| v <= MAX_VOLUME)
            .unwrap_or(MAX_VOLUME / 2);
        Ok(Self { station, volume })
    }

    pub fn save(self, nvs: EspDefaultNvsPartition) -> Result<()> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        nvs.set_u8("station", self.station)?;
        nvs.set_u8("volume", self.volume)?;
        Ok(())
    }
}
