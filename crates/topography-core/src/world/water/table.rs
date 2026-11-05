//! Per-class lake-to-catchment ratio r ≈ P/E (`configs/hydrology/water-v2.toml`), kept in
//! thousandths so supplies sum as integers, and two game settings: the smallest labelled
//! lake and the discharge-to-m³/s factor of the channels.

use crate::acquisition::{AcquisitionError, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The table built into the binary; `--water FILE` replaces it.
pub(crate) const DEFAULT: &str = include_str!("../../../../../configs/hydrology/water-v2.toml");
/// Supplies are counted in thousandths of a cell.
pub(crate) const MILLI: u64 = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Class {
    r: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u32,
    kind: String,
    // Required in the file (deny_unknown_fields) but not used by the model.
    #[allow(dead_code)]
    legend: String,
    lake_min_m2: f64,
    m3s_per_km2: f64,
    levels: BTreeMap<String, Level>,
    classes: BTreeMap<String, Class>,
}

/// Per-level thresholds (review N19): river start and breach cap.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Level {
    pub river_km2: f64,
    pub breach_cap_m: f32,
}

/// r in thousandths by class id (0 to 255; ids missing from the file take class 0), the game
/// settings and the per-level thresholds.
pub(crate) struct Table {
    r: Vec<u32>,
    /// Standing water is a labelled lake from this area (and 0.3 m deep).
    pub lake_min_m2: f64,
    /// m³/s per km² at mean rain, for hydraulic geometry.
    pub m3s_per_km2: f64,
    /// (spacing m, thresholds), ascending.
    levels: Vec<(f64, Level)>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)
            .map_err(|e| AcquisitionError::Invalid(format!("water table: {e}")))?;
        ensure(
            file.schema_version == 2 && file.kind == "water_table",
            "water table must be kind water_table, schema_version 2",
        )?;
        ensure(
            file.lake_min_m2.is_finite() && file.lake_min_m2 >= 0.0,
            "water table lake_min_m2 must be 0 or more",
        )?;
        ensure(
            file.m3s_per_km2.is_finite() && file.m3s_per_km2 > 0.0,
            "water table m3s_per_km2 must be positive",
        )?;
        let mut by_id = BTreeMap::new();
        for (key, class) in file.classes {
            let id: u8 = key
                .parse()
                .map_err(|_| AcquisitionError::Invalid(format!("water table class {key}")))?;
            ensure(
                class.r.is_finite() && (0.0..=100.0).contains(&class.r),
                format!("water table class {id}: r must be from 0 to 100"),
            )?;
            by_id.insert(id, (class.r * MILLI as f64).round() as u32);
        }
        let base = *by_id
            .get(&0)
            .ok_or_else(|| AcquisitionError::Invalid("water table needs class 0".into()))?;
        let mut levels = Vec::new();
        for (key, l) in file.levels {
            let spacing: f64 = key
                .parse()
                .map_err(|_| AcquisitionError::Invalid(format!("water table level {key}")))?;
            ensure(
                spacing.is_finite()
                    && spacing > 0.0
                    && l.river_km2.is_finite()
                    && l.river_km2 > 0.0
                    && l.breach_cap_m.is_finite()
                    && l.breach_cap_m >= 0.0,
                format!("water table level {key}: positive spacing and river start, cap 0 or more"),
            )?;
            levels.push((spacing, l));
        }
        levels.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure(!levels.is_empty(), "water table needs at least one level")?;
        Ok(Self {
            r: (0..=255u8)
                .map(|c| by_id.get(&c).copied().unwrap_or(base))
                .collect(),
            lake_min_m2: file.lake_min_m2,
            m3s_per_km2: file.m3s_per_km2,
            levels,
        })
    }

    /// r of a class in thousandths.
    pub fn r_milli(&self, class: u8) -> u32 {
        self.r[class as usize]
    }

    /// The thresholds of the largest level not coarser than `spacing_m` (the finest level
    /// for grids finer than every level).
    pub fn level(&self, spacing_m: f64) -> Level {
        debug_assert!(spacing_m.is_finite() && spacing_m > 0.0);
        self.levels
            .iter()
            .rev()
            .find(|l| l.0 <= spacing_m)
            .unwrap_or(&self.levels[0])
            .1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_table_parses_with_r_in_thousandths() {
        let t = Table::parse(DEFAULT).expect("built-in table");
        assert_eq!(t.r_milli(0), 1000);
        assert_eq!(t.r_milli(8), 0, "dunes are playas");
        assert_eq!(t.r_milli(7), 0);
        assert_eq!(t.r_milli(19), 50);
        assert_eq!(t.r_milli(2), 100);
        assert_eq!(t.r_milli(1), 20, "karst drains underground");
        assert_eq!(t.r_milli(17), 30);
        assert_eq!(t.r_milli(10), 1000);
        assert_eq!(t.r_milli(200), 1000, "missing classes take class 0");
        assert_eq!(t.lake_min_m2, 2000.0);
        assert_eq!(t.m3s_per_km2, 0.56);
        assert_eq!(
            t.level(1.0),
            Level {
                river_km2: 0.05,
                breach_cap_m: 2.0
            }
        );
        assert_eq!(
            t.level(4.0),
            Level {
                river_km2: 0.5,
                breach_cap_m: 5.0
            }
        );
        assert_eq!(
            t.level(32.0),
            Level {
                river_km2: 10.0,
                breach_cap_m: 20.0
            }
        );
        assert_eq!(
            t.level(2.0).breach_cap_m,
            2.0,
            "the largest level not coarser than the grid"
        );
    }

    #[test]
    fn a_v1_table_or_a_bad_value_is_refused() {
        let levels = "[levels.1]\nriver_km2 = 0.05\nbreach_cap_m = 2.0\n";
        let head = "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n";
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nr = 1.0\n")).is_ok());
        let v1 = format!(
            "schema_version = 1\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n{levels}[classes.0]\nr = 1.0\n"
        );
        assert!(Table::parse(&v1).is_err());
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nr = -1.0\n")).is_err());
        assert!(
            Table::parse(&format!("{head}{levels}[classes.3]\nr = 1.0\n")).is_err(),
            "no class 0"
        );
        assert!(
            Table::parse(&format!("{head}{levels}[classes.0]\nlake_share = 0.1\n")).is_err(),
            "v1 fields are unknown here"
        );
        assert!(
            Table::parse(&format!("{head}[classes.0]\nr = 1.0\n")).is_err(),
            "no levels"
        );
        let no_settings = format!(
            "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\n{levels}[classes.0]\nr = 1.0\n"
        );
        assert!(Table::parse(&no_settings).is_err());
        let zero = format!(
            "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.0\n{levels}[classes.0]\nr = 1.0\n"
        );
        assert!(
            Table::parse(&zero).is_err(),
            "the m³/s factor must be positive"
        );
    }

    #[test]
    fn r_above_100_or_a_class_key_beyond_u8_is_refused() {
        let head = "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n[levels.1]\nriver_km2 = 0.05\nbreach_cap_m = 2.0\n[classes.0]\nr = 1.0\n";
        assert!(Table::parse(&format!("{head}[classes.1]\nr = 100.5\n")).is_err());
        assert!(Table::parse(&format!("{head}[classes.256]\nr = 1.0\n")).is_err());
        assert!(Table::parse(&format!("{head}[classes.karst]\nr = 1.0\n")).is_err());
    }

    #[test]
    fn a_non_finite_level_threshold_is_refused() {
        let head = "schema_version = 2\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\n[classes.0]\nr = 1.0\n";
        let level = |river: &str, cap: &str| {
            format!("{head}[levels.1]\nriver_km2 = {river}\nbreach_cap_m = {cap}\n")
        };
        assert!(Table::parse(&level("0.05", "2.0")).is_ok());
        assert!(Table::parse(&level("inf", "2.0")).is_err());
        assert!(Table::parse(&level("0.05", "inf")).is_err());
        assert!(Table::parse(&level("nan", "2.0")).is_err());
    }
}
