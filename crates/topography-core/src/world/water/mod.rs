//! Hydrology v3: water as one field (ground, water surface, discharge) from one model at every
//! level. Exact graph passes on tiles: the priority-flood spill graph (Barnes 2016), the
//! depression hierarchy and Fill-Spill-Merge (Barnes, Callaghan and Wickert 2020), fall-line
//! receivers, flat ponds (Barnes 2014) and accumulation (Barnes 2017); at 1 m also channels.
//! The GPU bank relaxation runs in Python between `run` and `finish`.
//! See docs/superpowers/specs/2026-11-05-hydrology-v3-design.md.
// Items are wired to the CLI in Task 14, which removes this allow.
#![allow(dead_code)]

mod flow;
mod geom;
mod layers;
#[cfg(test)]
mod probe;
mod rain;
mod receivers;
mod store;
mod table;
mod watershed;

/// Judgment value (spec): standing water at least this deep is a lake; shallower is a pond.
pub(crate) const LAKE_DEPTH_M: f32 = 0.3;
/// Channel depth cap at 1 m; a 1 m breach plus its channel cuts at most this much too.
pub(crate) const CHANNEL_CAP_M: f32 = 2.0;
/// A gorge's bed falls over at least this many times its deepest cut (spec).
pub(crate) const GORGE_RATIO: f32 = 20.0;
/// Gorge cells lie within this many cells of their spill cell, so every tiling decides alike.
pub(crate) const BREACH_RADIUS: u64 = 256;
/// Judgment value: rivers from this size (km² at mean rain) are main rivers for the
/// one-thalweg check.
pub(crate) const MAIN_RIVER_KM2: f64 = 1.0;
/// Export ids are 16-bit (fewer than 65,535 water bodies); a run must stay well under that
/// (judgment value, review C2).
pub(crate) const BODY_BUDGET: usize = 50_000;
/// Default tile side in cells.
pub const DEFAULT_TILE: u64 = 4096;

/// A run's thresholds: the table's level for the grid spacing (review N19).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Params {
    /// River start, km² at mean rain.
    pub river_km2: f64,
    /// Deepest breach cut (at 1 m the channel cap: breach plus channel stay within it through
    /// the channels' raw − 2 m clamp).
    pub breach_cap_m: f32,
    /// Gorge cells lie within this many cells of their spill cell (tests lower it so a gorge
    /// crosses a tile seam).
    pub breach_radius: u64,
    /// Carve channels (1 m only).
    pub channels: bool,
}

impl Params {
    /// The `change_max` check's bound: this level's breach cap + 0.5 m.
    pub fn change_max_m(&self) -> f32 {
        self.breach_cap_m + 0.5
    }
}

pub(crate) fn params(table: &table::Table, spacing_m: f64, river_km2: Option<f64>) -> Params {
    let level = table.level(spacing_m);
    Params {
        river_km2: river_km2.unwrap_or(level.river_km2),
        breach_cap_m: level.breach_cap_m,
        breach_radius: BREACH_RADIUS,
        channels: spacing_m < 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_follow_the_spacing() {
        let t = table::Table::parse(table::DEFAULT).expect("table");
        let p = |s, r| params(&t, s, r);
        assert_eq!(
            p(32.0, None),
            Params {
                river_km2: 10.0,
                breach_cap_m: 20.0,
                breach_radius: BREACH_RADIUS,
                channels: false
            }
        );
        assert_eq!(
            p(4.0, None),
            Params {
                river_km2: 0.5,
                breach_cap_m: 5.0,
                breach_radius: BREACH_RADIUS,
                channels: false
            }
        );
        assert_eq!(
            p(1.0, None),
            Params {
                river_km2: 0.05,
                breach_cap_m: 2.0,
                breach_radius: BREACH_RADIUS,
                channels: true
            }
        );
        assert_eq!(p(1.0, Some(0.2)).river_km2, 0.2);
        assert_eq!(
            p(4.0, None).change_max_m(),
            5.5,
            "each level's own breach cap + 0.5 m"
        );
    }
}
