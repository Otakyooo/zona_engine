//! zn-content: данные зоны (биомы, POI, аномалии, артефакты, квесты).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneConfig {
    pub name: String,
    pub pois: Vec<String>,
}

pub fn load_zone_from_str(s: &str) -> Result<ZoneConfig, ron::error::SpannedError> {
    ron::from_str(s)
}
