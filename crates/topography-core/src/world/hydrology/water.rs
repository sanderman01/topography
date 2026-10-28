//! Per-class water amounts (`configs/hydrology/water-v1.toml`).

use crate::acquisition::{AcquisitionError, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The table built into the binary; `--water FILE` replaces it.
pub(super) const DEFAULT: &str = include_str!("../../../../../configs/hydrology/water-v1.toml");

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Water {
    pub lake_share: f64,
    pub tarn_share: f64,
    pub dry_share: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u32,
    kind: String,
    #[allow(dead_code)]
    legend: String,
    classes: BTreeMap<String, Water>,
}

/// Water by class id (0 to 255); ids missing from the file take class 0.
pub(super) struct Table(Vec<Water>);

impl Table {
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)
            .map_err(|e| AcquisitionError::Invalid(format!("water table: {e}")))?;
        ensure(
            file.schema_version == 1 && file.kind == "water_table",
            "water table must be kind water_table, schema_version 1",
        )?;
        let mut by_id = BTreeMap::new();
        for (key, water) in file.classes {
            let id: u8 = key
                .parse()
                .map_err(|_| AcquisitionError::Invalid(format!("water table class {key}")))?;
            ensure(
                [water.lake_share, water.dry_share]
                    .iter()
                    .all(|s| (0.0..=1.0).contains(s)),
                format!("water table class {id}: shares must be from 0 to 1"),
            )?;
            by_id.insert(id, water);
        }
        let base = *by_id
            .get(&0)
            .ok_or_else(|| AcquisitionError::Invalid("water table needs class 0".into()))?;
        Ok(Self(
            (0..=255u8)
                .map(|c| by_id.get(&c).copied().unwrap_or(base))
                .collect(),
        ))
    }

    pub fn get(&self, class: u8) -> &Water {
        &self.0[class as usize]
    }

    /// Budgets on a finer level: new lakes only as tarns (`tarn_share`, when `tarns`), no new
    /// dry basins.
    pub fn fine(&self, tarns: bool) -> Self {
        Self(
            self.0
                .iter()
                .map(|w| Water {
                    lake_share: if tarns { w.tarn_share } else { 0.0 },
                    dry_share: 0.0,
                    ..*w
                })
                .collect(),
        )
    }
}
