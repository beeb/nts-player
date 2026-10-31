use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};

use crate::{led::RING_LEDS, stream::STATION_COUNT};

const NAMESPACE: &str = "settings";

/// Volume step, from 0 (mute) to [`Volume::MAX`], one LED per step.
#[derive(Clone, Copy, Debug)]
pub struct Volume(u8);

impl Volume {
    pub const MAX: Self = Self(RING_LEDS);

    const DEFAULT: Self = Self(Self::MAX.0 / 2);

    /// Create a volume from `step`, stopping at [`Volume::MAX`].
    pub fn new(step: u8) -> Self {
        Self(step.min(Self::MAX.0))
    }

    /// One step louder, stopping at [`Volume::MAX`].
    pub fn up(self) -> Self {
        Self::new(self.0.saturating_add(1))
    }

    /// One step quieter, stopping at 0.
    pub fn down(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// User settings saved in NVS.
#[derive(Clone, Copy)]
pub struct Settings {
    /// Index into [`STATIONS`](crate::stream::STATIONS).
    pub station: u8,

    pub volume: Volume,
}

impl Settings {
    /// Load the settings from NVS, using defaults for missing or out-of-range values.
    pub fn load(nvs: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        let station = nvs
            .get_u8("station")?
            .filter(|&s| s < STATION_COUNT)
            .unwrap_or(0);
        let volume = nvs.get_u8("volume")?.map_or(Volume::DEFAULT, Volume::new);
        Ok(Self { station, volume })
    }

    pub fn save(self, nvs: EspDefaultNvsPartition) -> Result<()> {
        let nvs = EspNvs::new(nvs, NAMESPACE, true)?;
        nvs.set_u8("station", self.station)?;
        nvs.set_u8("volume", self.volume.get())?;
        Ok(())
    }
}
