//! Validation checks, figures and the body list.

use super::flood::{Dims, RIVER, SEA, receiver};
use super::lakes::{Basin, Depression, Kind};
use super::rivers::{End, River};
use serde_json::{Map, Value, json};

pub(super) struct Context<'a> {
    pub dims: Dims,
    pub ground: &'a [f32],
    pub dir: &'a [u8],
    pub acc: &'a [f32],
    pub flags: &'a [u8],
    /// Basin id, or river id where `RIVER` is set.
    pub body: &'a [u32],
    pub surface: &'a [f32],
    pub basins: &'a [Basin],
    pub deps: &'a [Depression],
    pub rivers: &'a [River],
    pub cell_km2: f64,
    pub spacing_m: f64,
}

impl Context<'_> {
    fn sea(&self, i: usize) -> bool {
        self.flags[i] & SEA != 0
    }
    fn river(&self, i: usize) -> bool {
        self.flags[i] & RIVER != 0
    }
    fn basin(&self, i: usize) -> Option<&Basin> {
        (!self.sea(i) && !self.river(i) && self.body[i] != 0)
            .then(|| &self.basins[self.body[i] as usize - 1])
    }
    fn lake(&self, i: usize) -> Option<u32> {
        self.basin(i)
            .filter(|b| b.kind == Kind::Lake)
            .map(|_| self.body[i])
    }
}

/// Ground lowered by breaching, against the input.
#[derive(Default)]
pub(super) struct Carve {
    cells: u64,
    /// Cut more than 50 m: slots through ridges, worth a look.
    deep: u64,
    volume_m3: f64,
    max_m: f32,
}

impl Carve {
    pub fn add(&mut self, input: &[f32], carved: &[f32], spacing_m: f64) {
        for (a, b) in input.iter().zip(carved) {
            if b < a {
                self.cells += 1;
                if a - b > 50.0 {
                    self.deep += 1;
                }
                self.volume_m3 += (a - b) as f64 * spacing_m * spacing_m;
                self.max_m = self.max_m.max(a - b);
            }
        }
    }
}

/// Hard checks; every value must be 0.
pub(super) fn checks(c: &Context, cycles: u64) -> Map<String, Value> {
    let n = c.ground.len();
    let (mut undrained, mut uphill, mut not_above, mut rises) = (cycles, 0u64, 0u64, 0u64);
    let mut lake_range = vec![(f32::INFINITY, f32::NEG_INFINITY); c.basins.len() + 1];
    for i in 0..n {
        let r = receiver(c.dims, c.dir, i);
        if r.is_none() && !c.sea(i) && !c.dims.is_edge(i) {
            undrained += 1;
        }
        let land = !c.sea(i) && c.basin(i).is_none();
        if let Some(r) = r {
            if land && !c.sea(r) && c.basin(r).is_none() && c.ground[r] > c.ground[i] {
                uphill += 1;
            }
        }
        let s = c.surface[i];
        if !s.is_nan() && s <= c.ground[i] {
            not_above += 1;
        }
        if let Some(l) = c.lake(i) {
            let range = &mut lake_range[l as usize];
            *range = (range.0.min(s), range.1.max(s));
        }
        if c.river(i) {
            let below = match r {
                Some(r) if c.river(r) || c.sea(r) || c.lake(r).is_some() => c.surface[r],
                _ => f32::NEG_INFINITY,
            };
            if below > s {
                rises += 1;
            }
        }
    }
    let several = lake_range
        .iter()
        .filter(|(lo, hi)| lo.is_finite() && lo != hi)
        .count();
    let on_land = c.rivers.iter().filter(|r| r.end == End::Land).count();
    let mut m = Map::new();
    m.insert(
        "land_cells_not_draining_to_an_outlet".into(),
        json!(undrained),
    );
    m.insert("land_cells_draining_uphill".into(), json!(uphill));
    m.insert("lakes_with_several_levels".into(), json!(several));
    m.insert(
        "wet_cells_with_surface_not_above_ground".into(),
        json!(not_above),
    );
    m.insert("river_cells_rising_downstream".into(), json!(rises));
    m.insert("rivers_ending_on_dry_land".into(), json!(on_land));
    m
}

pub(super) fn figures(c: &Context, carve: &Carve, extended: u64) -> Value {
    let n = c.ground.len();
    let sea = (0..n).filter(|&i| c.sea(i)).count();
    let lake_cells = (0..n).filter(|&i| c.lake(i).is_some()).count();
    let dry_cells = (0..n)
        .filter(|&i| c.basin(i).is_some_and(|b| b.kind == Kind::Dry))
        .count();
    let river_cells = (0..n).filter(|&i| c.river(i)).count();
    // Lake water touching lower ground outside the lake (a leak in the rim).
    let leaks = (0..n)
        .filter(|&i| {
            c.lake(i).is_none()
                && c.dims.neighbours(i).any(|(_, j)| {
                    c.lake(j)
                        .is_some_and(|_| c.ground[i] < c.surface[j] && !c.river(i))
                })
        })
        .count();
    let lakes = c.basins.iter().filter(|b| b.kind == Kind::Lake).count();
    let land = (n - sea) as f64;
    let ends = |e: fn(&End) -> bool| c.rivers.iter().filter(|r| e(&r.end)).count();
    let km2 = |cells: usize| cells as f64 * c.cell_km2;
    json!({
        "cells": n,
        "spacing_m": c.spacing_m,
        "sea_km2": km2(sea),
        "sea_share": sea as f64 / n as f64,
        "land_km2": km2(n - sea),
        "depressions": c.deps.len(),
        "lakes": lakes,
        "lake_km2": km2(lake_cells),
        "lake_share_of_land": lake_cells as f64 / land.max(1.0),
        "lakes_filled_by_river": c.basins.iter().filter(|b| b.filled_by_river).count(),
        "lake_cells_added_below_level": extended,
        "dry_land_cells_below_adjacent_lake_level": leaks,
        "dry_basins": c.basins.len() - lakes,
        "dry_basin_km2": km2(dry_cells),
        "breached": c.deps.len() - c.basins.len(),
        "carved_cells": carve.cells,
        "carved_cells_over_50m": carve.deep,
        "carve_volume_m3": carve.volume_m3.round(),
        "max_cut_m": carve.max_m,
        "river_systems": c.rivers.len(),
        "river_cells": river_cells,
        "river_length_km": c.rivers.iter().map(|r| r.length_m).sum::<f64>() / 1000.0,
        "rivers_to_sea": ends(|e| *e == End::Sea),
        "rivers_to_lakes": ends(|e| matches!(e, End::Lake(_))),
        "rivers_to_grid_edge": ends(|e| *e == End::Edge),
        "max_drainage_km2": c.acc.iter().zip(c.flags).filter(|(_, f)| **f & SEA == 0)
            .map(|(a, _)| *a).fold(0f32, f32::max),
    })
}

pub(super) fn constants(river_km2: f64) -> Value {
    json!({
        "sea": "ground below 0 m connected 8-way to the grid edge; surface 0",
        "river_start_km2": river_km2,
        "river_depth": "0.4 m * drainage_km2^0.4, raised so the surface never rises downstream",
        "lake_min_depth_m": 1.0,
        "lake_min_area_m2": 4096.0,
        "dry_basin_min_depth_m": 2.0,
        "dry_basin_max_elongation": 4.0,
        "lake_order": "largest volume first within each class's lake_share of its land",
        "dry_order": "deepest first within each dry class's dry_share, only without river inflow",
        "breach": "priority-flood carving, f32 steps down along the cut",
    })
}

/// `bodies.json`: sea, lakes, rivers with their parent body (where their water goes; 0 for
/// the grid edge).
pub(super) fn bodies(
    c: &Context,
    body_id: &dyn Fn(usize) -> u32,
    lake_id: &[u32],
    river_base: u32,
) -> Value {
    let cell = |i: u32| json!([i as usize % c.dims.w, i as usize / c.dims.w]);
    let mut lake_cells = vec![0u64; c.basins.len() + 1];
    let mut lake_depth = vec![0f32; c.basins.len() + 1];
    let mut sea = 0u64;
    for i in 0..c.ground.len() {
        if c.sea(i) {
            sea += 1;
        } else if let Some(l) = c.lake(i) {
            lake_cells[l as usize] += 1;
            let d = &mut lake_depth[l as usize];
            *d = d.max(c.surface[i] - c.ground[i]);
        }
    }
    // Downstream from a lake exit to the first water body.
    let parent = |mut i: usize| loop {
        let id = body_id(i);
        if id != 0 {
            return id;
        }
        match receiver(c.dims, c.dir, i) {
            Some(r) => i = r,
            None => return 0,
        }
    };
    let mut list = vec![json!({
        "id": 1, "kind": "sea", "level_m": 0.0, "area_km2": sea as f64 * c.cell_km2,
    })];
    for (b, basin) in c.basins.iter().enumerate() {
        if basin.kind != Kind::Lake {
            continue;
        }
        list.push(json!({
            "id": lake_id[b + 1],
            "kind": "lake",
            "level_m": basin.level,
            "outlet_cell": cell(basin.exit),
            "area_km2": lake_cells[b + 1] as f64 * c.cell_km2,
            "max_depth_m": lake_depth[b + 1],
            "filled_by_river": basin.filled_by_river,
            "parent": parent(basin.exit as usize),
        }));
    }
    for (r, river) in c.rivers.iter().enumerate() {
        let parent = match river.end {
            End::Sea => 1,
            End::Lake(b) => lake_id[b as usize],
            End::Edge | End::Land => 0,
        };
        list.push(json!({
            "id": river_base + r as u32 + 1,
            "kind": "river",
            "mouth_cell": cell(river.mouth),
            "drainage_km2": river.drainage_km2,
            "length_km": river.length_m / 1000.0,
            "cells": river.cells,
            "parent": parent,
        }));
    }
    json!({
        "schema_version": 1,
        "kind": "water_bodies",
        "cells": "[column, row] from the north-west corner",
        "bodies": list,
    })
}
