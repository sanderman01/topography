//! Per-class lake-to-catchment ratio r ≈ P/E and hot-dry flag (`configs/hydrology/water-v3.toml`),
//! r kept in thousandths so supplies sum as integers; the game settings (smallest labelled
//! lake, discharge-to-m³/s factor, below-sea coast distance) and per-level thresholds.

use crate::acquisition::{AcquisitionError, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The table built into the binary; `--water FILE` replaces it.
pub(crate) const DEFAULT: &str = include_str!("../../../../../configs/hydrology/water-v3.toml");
/// Supplies are counted in thousandths of a cell.
pub(crate) const MILLI: u64 = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Class {
    r: f64,
    #[serde(default)]
    hot_dry: bool,
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
    below_sea_coast_km: f64,
    levels: BTreeMap<String, Level>,
    classes: BTreeMap<String, Class>,
}

/// Coarse-level erosion (rulings B2–B7).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Erosion {
    pub routings: u32,
    pub k_dt: f64,
    pub m: f64,
    /// Mass-conserving diffusion: alpha = D·dt/dx² per routing (explicit, at most 0.25).
    pub diffusion: f32,
    pub cap_m: f32,
    pub interior_k: f64,
    pub interior_cap_m: f32,
}

/// Per-level thresholds (review N19, rulings B4, B8).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Level {
    pub river_km2: f64,
    pub breach_cap_m: f32,
    pub wall_deg: f64,
    #[serde(default)]
    pub erosion: Option<Erosion>,
    #[serde(default)]
    pub climate: Option<Climate>,
}

/// Climate from the moisture field (owner ruling 2026-11-11, option A and I1): lake supply
/// follows rain, lakes evaporate more where it is dry, rivers carry runoff, glaciers hold ice.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Climate {
    /// Rain (relative to the mean land rain) lost to evapotranspiration before any runoff.
    pub runoff_threshold: f64,
    /// Lake evaporation (relative to a humid lake) at or below `moist_dry`.
    pub evap_dry: f64,
    /// Moisture (0 to 1) at or below which lakes evaporate `evap_dry` ...
    pub moist_dry: f64,
    /// ... and at or above which they evaporate 1; linear between.
    pub moist_wet: f64,
    /// Standing water whose cells, or whose shore ring, are more than this share cold
    /// (snow/ice or glacier class 10) is ice.
    pub ice_share: f64,
    /// Share of the runoff and lake supply a cold cell gives: precipitation there falls as
    /// snow and only meltwater leaves.
    pub snow_melt: f64,
    /// Blocks of this size (m, aligned to the grid) whose snow/ice share is at least
    /// `cold_share` are cold throughout: air temperature, not slope, sets the precipitation
    /// phase, so steep rock that sheds its snow is cold too.
    pub cold_block_m: f64,
    pub cold_share: f64,
}

/// The glacier class of legend terrain-v3.
pub(crate) const GLACIER: u8 = 10;

/// `snow-ice.u8` value for snow/ice present (1: absent).
pub(crate) const SNOW_PRESENT: u8 = 2;

impl Climate {
    /// Runoff relative to the mean land rain's: rain above the threshold, scaled so mean
    /// rain gives 1.
    pub fn runoff(&self, rain: f64) -> f64 {
        (rain - self.runoff_threshold).max(0.0) / (1.0 - self.runoff_threshold)
    }

    /// Lake evaporation in thousandths of a humid lake cell's, at a moisture.
    pub fn evap_milli(&self, moisture: f64) -> u64 {
        let t = ((moisture - self.moist_dry) / (self.moist_wet - self.moist_dry)).clamp(0.0, 1.0);
        (MILLI as f64 * (self.evap_dry + (1.0 - self.evap_dry) * t)).round() as u64
    }

    fn valid(&self) -> bool {
        (0.0..1.0).contains(&self.runoff_threshold)
            && self.evap_dry >= 1.0
            && self.evap_dry <= 100.0
            && 0.0 <= self.moist_dry
            && self.moist_dry < self.moist_wet
            && self.moist_wet <= 1.0
            && (0.0..1.0).contains(&self.ice_share)
            && (0.0..=1.0).contains(&self.snow_melt)
            && self.cold_block_m.is_finite()
            && self.cold_block_m > 0.0
            && self.cold_share > 0.0
            && self.cold_share <= 1.0
    }
}

/// r in thousandths and the hot-dry flag by class id (0 to 255; ids missing from the file take
/// class 0), the game settings and the per-level thresholds.
pub(crate) struct Table {
    r: Vec<u32>,
    // Read by the coastal water table (B13) from a later Plan B task.
    #[allow(dead_code)]
    hot: Vec<bool>,
    /// Standing water is a labelled lake from this area (and 0.3 m deep).
    pub lake_min_m2: f64,
    /// m³/s per km² at mean rain, for hydraulic geometry.
    pub m3s_per_km2: f64,
    /// A hot-dry basin keeps a lake below 0 m only this far from open sea (B13).
    #[allow(dead_code)]
    pub below_sea_coast_km: f64,
    /// (spacing m, thresholds), ascending.
    levels: Vec<(f64, Level)>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)
            .map_err(|e| AcquisitionError::Invalid(format!("water table: {e}")))?;
        ensure(
            file.schema_version == 3 && file.kind == "water_table",
            "water table must be kind water_table, schema_version 3",
        )?;
        ensure(
            file.lake_min_m2.is_finite() && file.lake_min_m2 >= 0.0,
            "water table lake_min_m2 must be 0 or more",
        )?;
        ensure(
            file.m3s_per_km2.is_finite() && file.m3s_per_km2 > 0.0,
            "water table m3s_per_km2 must be positive",
        )?;
        ensure(
            file.below_sea_coast_km.is_finite() && file.below_sea_coast_km >= 0.0,
            "water table below_sea_coast_km must be 0 or more",
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
            by_id.insert(id, ((class.r * MILLI as f64).round() as u32, class.hot_dry));
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
                    && l.breach_cap_m >= 0.0
                    && l.wall_deg > 0.0
                    && l.wall_deg < 90.0,
                format!(
                    "water table level {key}: positive spacing and river start, cap 0 or more, \
                     0 < wall_deg < 90"
                ),
            )?;
            ensure(
                l.climate.is_none_or(|c| c.valid()),
                format!(
                    "water table level {key}: climate needs 0 <= runoff_threshold < 1, \
                     1 <= evap_dry <= 100, 0 <= moist_dry < moist_wet <= 1, 0 <= ice_share < 1, \
                     0 <= snow_melt <= 1, cold_block_m > 0, 0 < cold_share <= 1"
                ),
            )?;
            if let Some(e) = &l.erosion {
                ensure(
                    e.routings >= 1
                        && e.k_dt.is_finite()
                        && e.k_dt >= 0.0
                        && e.m > 0.0
                        && e.m <= 1.0
                        && (0.0..=0.25).contains(&e.diffusion)
                        && e.cap_m >= 0.0
                        && e.cap_m.is_finite()
                        && e.interior_cap_m >= 0.0
                        && e.interior_cap_m.is_finite()
                        && (0.0..=1.0).contains(&e.interior_k),
                    format!(
                        "water table level {key}: erosion routings >= 1, k_dt >= 0, 0 < m <= 1, \
                         0 <= diffusion <= 0.25, caps >= 0, 0 <= interior_k <= 1"
                    ),
                )?;
            }
            levels.push((spacing, l));
        }
        levels.sort_by(|a, b| a.0.total_cmp(&b.0));
        ensure(!levels.is_empty(), "water table needs at least one level")?;
        let class = |c: u8| by_id.get(&c).copied().unwrap_or(base);
        Ok(Self {
            r: (0..=255u8).map(|c| class(c).0).collect(),
            hot: (0..=255u8).map(|c| class(c).1).collect(),
            lake_min_m2: file.lake_min_m2,
            m3s_per_km2: file.m3s_per_km2,
            below_sea_coast_km: file.below_sea_coast_km,
            levels,
        })
    }

    /// r of a class in thousandths.
    pub fn r_milli(&self, class: u8) -> u32 {
        self.r[class as usize]
    }

    /// The class may keep a lake below 0 m far from the sea (ruling B13).
    #[allow(dead_code)]
    pub fn hot_dry(&self, class: u8) -> bool {
        self.hot[class as usize]
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
    fn the_built_in_table_parses_with_r_levels_and_erosion() {
        let t = Table::parse(DEFAULT).expect("built-in table");
        assert_eq!(t.r_milli(0), 1000);
        assert_eq!(t.r_milli(8), 0, "dunes are playas");
        assert_eq!(t.r_milli(19), 50);
        assert_eq!(t.r_milli(200), 1000, "missing classes take class 0");
        assert!(t.hot_dry(19) && t.hot_dry(8) && t.hot_dry(7) && t.hot_dry(2));
        assert!(
            !t.hot_dry(1) && !t.hot_dry(17) && !t.hot_dry(0),
            "karst and lava are not hot-dry"
        );
        assert_eq!(t.below_sea_coast_km, 5.0);
        let l1 = t.level(1.0);
        assert_eq!(
            (l1.river_km2, l1.breach_cap_m, l1.wall_deg),
            (0.05, 2.0, 40.0)
        );
        assert!(l1.erosion.is_none(), "no erosion at 1 m");
        let c = l1.climate.expect("1 m has a climate");
        assert!(t.level(4.0).climate.is_none() && t.level(32.0).climate.is_none());
        assert_eq!(c.runoff(c.runoff_threshold), 0.0);
        assert!(
            (c.runoff(1.0) - 1.0).abs() < 1e-12,
            "mean rain gives mean runoff"
        );
        assert_eq!(c.evap_milli(0.0), (c.evap_dry * 1000.0).round() as u64);
        assert_eq!(c.evap_milli(c.moist_wet), 1000);
        assert_eq!(c.evap_milli(1.0), 1000);
        assert!(
            c.snow_melt > 0.0 && c.snow_melt < 0.5,
            "snow gives a little meltwater"
        );
        let bad = DEFAULT.replace("snow_melt = 0.15", "snow_melt = 1.5");
        assert!(Table::parse(&bad).is_err(), "snow_melt is a share");
        let e4 = t.level(4.0).erosion.expect("4 m erodes");
        assert_eq!((e4.routings, e4.cap_m, e4.interior_cap_m), (2, 10.0, 3.0));
        assert_eq!(t.level(4.0).wall_deg, 35.0);
        let e32 = t.level(32.0).erosion.expect("32 m erodes");
        assert_eq!((e32.routings, e32.k_dt, e32.cap_m), (5, 7e-4, 30.0));
    }

    #[test]
    fn schema_2_tables_and_bad_erosion_are_refused() {
        let v2 = DEFAULT.replace("schema_version = 3", "schema_version = 2");
        assert!(Table::parse(&v2).is_err());
        let bad = DEFAULT.replace("routings = 2", "routings = 0");
        assert!(Table::parse(&bad).is_err(), "at least one routing");
        let bad = DEFAULT.replace(
            "diffusion = 0.05\ncap_m = 10.0",
            "diffusion = 0.3\ncap_m = 10.0",
        );
        assert!(
            Table::parse(&bad).is_err(),
            "explicit diffusion needs alpha <= 0.25"
        );
    }

    #[test]
    fn a_v1_table_or_a_bad_value_is_refused() {
        let levels = "[levels.1]\nriver_km2 = 0.05\nbreach_cap_m = 2.0\nwall_deg = 40.0\n";
        let head = "schema_version = 3\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\nbelow_sea_coast_km = 5.0\n";
        assert!(Table::parse(&format!("{head}{levels}[classes.0]\nr = 1.0\n")).is_ok());
        let v1 = format!(
            "schema_version = 1\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\nbelow_sea_coast_km = 5.0\n{levels}[classes.0]\nr = 1.0\n"
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
            "schema_version = 3\nkind = \"water_table\"\nlegend = \"x\"\n{levels}[classes.0]\nr = 1.0\n"
        );
        assert!(Table::parse(&no_settings).is_err());
        let zero = format!(
            "schema_version = 3\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.0\nbelow_sea_coast_km = 5.0\n{levels}[classes.0]\nr = 1.0\n"
        );
        assert!(
            Table::parse(&zero).is_err(),
            "the m³/s factor must be positive"
        );
    }

    #[test]
    fn r_above_100_or_a_class_key_beyond_u8_is_refused() {
        let head = "schema_version = 3\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\nbelow_sea_coast_km = 5.0\n[levels.1]\nriver_km2 = 0.05\nbreach_cap_m = 2.0\nwall_deg = 40.0\n[classes.0]\nr = 1.0\n";
        assert!(Table::parse(&format!("{head}[classes.1]\nr = 100.5\n")).is_err());
        assert!(Table::parse(&format!("{head}[classes.256]\nr = 1.0\n")).is_err());
        assert!(Table::parse(&format!("{head}[classes.karst]\nr = 1.0\n")).is_err());
    }

    #[test]
    fn a_non_finite_level_threshold_is_refused() {
        let head = "schema_version = 3\nkind = \"water_table\"\nlegend = \"x\"\nlake_min_m2 = 500.0\nm3s_per_km2 = 0.5\nbelow_sea_coast_km = 5.0\n[classes.0]\nr = 1.0\n";
        let level = |river: &str, cap: &str| {
            format!(
                "{head}[levels.1]\nriver_km2 = {river}\nbreach_cap_m = {cap}\nwall_deg = 40.0\n"
            )
        };
        assert!(Table::parse(&level("0.05", "2.0")).is_ok());
        assert!(Table::parse(&level("inf", "2.0")).is_err());
        assert!(Table::parse(&level("0.05", "inf")).is_err());
        assert!(Table::parse(&level("nan", "2.0")).is_err());
    }
}
