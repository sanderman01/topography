//! Validation checks, figures and the body list.

use super::flood::{Dims, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind};
use super::rivers::{End, River, entered_basin, sill_level};
use serde_json::{Map, Value, json};

/// Dry ground may sit this far below adjacent water (f32 rounding of carved rims).
const BANK_TOLERANCE_M: f32 = 0.01;

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
    fn wet(&self, i: usize) -> bool {
        self.sea(i) || self.river(i) || self.lake(i).is_some()
    }
}

/// Ground lowered by breaching and river beds, against the input.
#[derive(Default)]
pub(super) struct Carve {
    cells: u64,
    /// Cut more than 10 m.
    deep: u64,
    volume_m3: f64,
    max_m: f32,
}

impl Carve {
    pub fn add(&mut self, input: &[f32], carved: &[f32], spacing_m: f64) {
        for (a, b) in input.iter().zip(carved) {
            if b < a {
                self.cells += 1;
                if a - b > 10.0 {
                    self.deep += 1;
                }
                self.volume_m3 += (a - b) as f64 * spacing_m * spacing_m;
                self.max_m = self.max_m.max(a - b);
            }
        }
    }
}

/// Hard checks; every value must be 0. `guard_` checks hold by construction and only catch
/// bugs; the others test the water contract on the outputs.
pub(super) fn checks(c: &Context, cycles: u64) -> Map<String, Value> {
    let n = c.ground.len();
    let mut undrained = cycles;
    let (mut uphill, mut not_above, mut rises, mut not_below_level) = (0u64, 0u64, 0u64, 0u64);
    let (mut below_lake, mut below_sea, mut below_river) = (0u64, 0u64, 0u64);
    let mut river_below_lake = 0u64;
    let mut lake_range = vec![(f32::INFINITY, f32::NEG_INFINITY); c.basins.len() + 1];
    for i in 0..n {
        let r = receiver(c.dims, c.dir, i);
        let sink = c.basin(i).is_some_and(|b| b.kind == Kind::Sink);
        if r.is_none() && !c.sea(i) && !c.dims.is_edge(i) && !sink {
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
            if c.ground[i] >= c.basins[l as usize - 1].level {
                not_below_level += 1;
            }
        }
        if c.river(i) {
            let below = match r {
                Some(r) if c.wet(r) => c.surface[r],
                _ => f32::NEG_INFINITY,
            };
            if below > s {
                rises += 1;
            }
        }
        if !c.wet(i) {
            let g = c.ground[i] + BANK_TOLERANCE_M;
            let next = |kind: fn(&Context, usize) -> bool| {
                c.dims
                    .neighbours(i)
                    .any(|(_, j)| kind(c, j) && c.surface[j] > g)
            };
            below_lake += next(|c, j| c.lake(j).is_some()) as u64;
            below_sea += next(|c, j| c.sea(j)) as u64;
            // A lake exit's banks may drop away (waterfall lip); the basin a river enters is
            // not its bank.
            below_river += c.dims.neighbours(i).any(|(_, j)| {
                c.river(j)
                    && c.surface[j] > g
                    && sill_level(c.dims, c.dir, c.flags, c.body, c.basins, j).is_none()
                    && (c.body[i] == 0
                        || c.body[i] != entered_basin(c.dims, c.dir, c.flags, c.body, j))
            }) as u64;
        }
        if c.river(i) {
            let low = s + BANK_TOLERANCE_M;
            river_below_lake +=
                c.dims
                    .neighbours(i)
                    .any(|(_, j)| c.lake(j).is_some() && c.surface[j] > low) as u64;
        }
    }
    let several = lake_range
        .iter()
        .filter(|(lo, hi)| lo.is_finite() && lo != hi)
        .count();
    // Lake exits: an outflow river at the level (a sill), dry ground not above it.
    let exits_off = c
        .basins
        .iter()
        .filter(|b| b.kind == Kind::Lake)
        .filter(|b| {
            let e = b.exit as usize;
            if c.sea(e) || c.lake(e).is_some() {
                false
            } else if c.river(e) {
                (c.surface[e] - b.level).abs() > BANK_TOLERANCE_M
            } else {
                c.ground[e] > b.level + BANK_TOLERANCE_M
            }
        })
        .count();
    let on_land = c.rivers.iter().filter(|r| r.end == End::Land).count();
    let mut m = Map::new();
    m.insert(
        "guard_land_cells_not_draining_to_an_outlet".into(),
        json!(undrained),
    );
    m.insert("guard_lakes_with_several_levels".into(), json!(several));
    m.insert("land_cells_draining_uphill".into(), json!(uphill));
    m.insert("lakes_in_several_pieces".into(), json!(lake_pieces(c)));
    m.insert("lake_cells_not_below_level".into(), json!(not_below_level));
    m.insert("lake_exits_off_level".into(), json!(exits_off));
    m.insert(
        "river_cells_below_adjacent_lake".into(),
        json!(river_below_lake),
    );
    m.insert(
        "wet_cells_with_surface_not_above_ground".into(),
        json!(not_above),
    );
    m.insert("river_cells_rising_downstream".into(), json!(rises));
    m.insert("rivers_ending_on_dry_land".into(), json!(on_land));
    m.insert("dry_cells_below_adjacent_lake".into(), json!(below_lake));
    m.insert("dry_cells_below_adjacent_sea".into(), json!(below_sea));
    m.insert("dry_cells_below_adjacent_river".into(), json!(below_river));
    m
}

/// Lakes whose cells form more than one 8-connected piece.
fn lake_pieces(c: &Context) -> usize {
    let n = c.ground.len();
    let mut seen = vec![false; n];
    let mut pieces = vec![0u32; c.basins.len() + 1];
    let mut stack = Vec::new();
    for i in 0..n {
        let Some(id) = c.lake(i).filter(|_| !seen[i]) else {
            continue;
        };
        pieces[id as usize] += 1;
        seen[i] = true;
        stack.push(i);
        while let Some(a) = stack.pop() {
            for (_, j) in c.dims.neighbours(a) {
                if !seen[j] && c.lake(j) == Some(id) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    pieces.iter().filter(|&&p| p > 1).count()
}

pub(super) fn figures(c: &Context, carve: &Carve, extended: u64) -> Map<String, Value> {
    let n = c.ground.len();
    let sea = (0..n).filter(|&i| c.sea(i)).count();
    let lake_cells = (0..n).filter(|&i| c.lake(i).is_some()).count();
    let dry_cells = (0..n)
        .filter(|&i| c.basin(i).is_some_and(|b| b.kind != Kind::Lake))
        .count();
    let river_cells = (0..n).filter(|&i| c.river(i)).count();
    let count = |f: fn(&Basin) -> bool| c.basins.iter().filter(|b| f(b)).count();
    let land = (n - sea) as f64;
    let ends = |e: fn(&End) -> bool| c.rivers.iter().filter(|r| e(&r.end)).count();
    let km2 = |cells: usize| cells as f64 * c.cell_km2;
    let value = json!({
        "cells": n,
        "spacing_m": c.spacing_m,
        "sea_km2": km2(sea),
        "sea_share": sea as f64 / n as f64,
        "land_km2": km2(n - sea),
        "lakes": count(|b| b.kind == Kind::Lake),
        "lake_km2": km2(lake_cells),
        "lake_share_of_land": lake_cells as f64 / land.max(1.0),
        "over_budget_lakes": count(|b| b.kind == Kind::Lake && b.over_budget),
        "lakes_filled_by_river": count(|b| b.filled_by_river),
        "lake_cells_added_below_level": extended,
        "dry_basins": count(|b| b.kind != Kind::Lake),
        "over_budget_sinks": count(|b| b.kind == Kind::Sink),
        "ice_basins": count(|b| b.ice),
        "dry_basin_km2": km2(dry_cells),
        "carved_cells": carve.cells,
        "carved_cells_over_10m": carve.deep,
        "carve_volume_m3": carve.volume_m3.round(),
        "max_cut_m": carve.max_m,
        "river_systems": c.rivers.len(),
        "river_cells": river_cells,
        "river_length_km": c.rivers.iter().map(|r| r.length_m).sum::<f64>() / 1000.0,
        "rivers_to_sea": ends(|e| *e == End::Sea),
        "rivers_to_lakes": ends(|e| matches!(e, End::Lake(_))),
        "rivers_to_sinks": ends(|e| matches!(e, End::Sink(_))),
        "rivers_into_ice_basins": ends(|e| matches!(e, End::Dry(_))),
        "rivers_to_grid_edge": ends(|e| *e == End::Edge),
        "max_drainage_km2": c.acc.iter().zip(c.flags).filter(|(_, f)| **f & SEA == 0)
            .map(|(a, _)| *a).fold(0f32, f32::max),
    });
    match value {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub(super) fn constants(river_km2: f64, breach_max_m: f32) -> Value {
    json!({
        "sea": "input ground below 0 m connected 8-way to the grid edge; surface 0",
        "river_start_km2": river_km2,
        "river_depth": "0.4 m * drainage_km2^0.4; surface at most the lowest dry bank and every \
            surface upstream, bed cut to surface minus depth",
        "breach_max_m": breach_max_m,
        "deep_depressions": "kept beyond the budget: dry sinks in glacier and arid classes \
            (lake_share <= 0.002), else lakes at the spill lowered by breach_max_m, not below 0 m",
        "sub_zero_depressions": "at most breach_max_m deep with ground below 0 m: lakes at 0 m \
            (lagoons) in any class",
        "lake_exit": "an outflow river starts at the lake level; its banks may drop away",
        "lake_min_depth_m": 1.0,
        "lake_min_area_m2": 4096.0,
        "dry_basin_min_depth_m": 2.0,
        "dry_basin_max_elongation": 4.0,
        "lake_order": "largest volume first within each class's lake_share of its land",
        "dry_order": "deepest first within each dry class's dry_share, only without river inflow",
        "glacier_basins": "class 10 depressions stay dry ice basins, never lakes or breached",
        "bank_tolerance_m": BANK_TOLERANCE_M,
    })
}

/// `bodies.json`: sea, lakes and rivers (ids as in `water-body.u32`), then dry basins (ids
/// not in the raster) with their parent body (where their water goes; 0 for the grid edge).
pub(super) fn bodies(
    c: &Context,
    body_id: &dyn Fn(usize) -> u32,
    basin_id: &[u32],
    river_base: u32,
) -> Value {
    let cell = |i: u32| json!([i as usize % c.dims.w, i as usize / c.dims.w]);
    let mut cells = vec![0u64; c.basins.len() + 1];
    let mut depth = vec![0f32; c.basins.len() + 1];
    let mut sea = 0u64;
    for i in 0..c.ground.len() {
        if c.sea(i) {
            sea += 1;
        } else if let Some(b) = c.basin(i) {
            let k = c.body[i] as usize;
            cells[k] += 1;
            depth[k] = depth[k].max(b.level - c.ground[i]);
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
    let mut dry = Vec::new();
    for (b, basin) in c.basins.iter().enumerate() {
        let common = json!({
            "id": basin_id[b + 1],
            "level_m": basin.level,
            "area_km2": cells[b + 1] as f64 * c.cell_km2,
            "max_depth_m": depth[b + 1],
            "over_budget": basin.over_budget,
        });
        let mut entry = match common {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if basin.kind == Kind::Lake {
            entry.insert("kind".into(), json!("lake"));
            entry.insert("exit_cell".into(), cell(basin.exit));
            entry.insert("filled_by_river".into(), json!(basin.filled_by_river));
            entry.insert("parent".into(), json!(parent(basin.exit as usize)));
            list.push(Value::Object(entry));
        } else {
            entry.insert("kind".into(), json!("dry"));
            entry.insert("endorheic".into(), json!(basin.kind == Kind::Sink));
            entry.insert("ice".into(), json!(basin.ice));
            dry.push(Value::Object(entry));
        }
    }
    for (r, river) in c.rivers.iter().enumerate() {
        let parent = match river.end {
            End::Sea => 1,
            End::Lake(b) | End::Sink(b) | End::Dry(b) => basin_id[b as usize],
            End::Edge | End::Land => 0,
        };
        list.push(json!({
            "id": river_base + r as u32 + 1,
            "kind": "river",
            "mouth_cell": cell(river.mouth),
            "area_km2": river.cells as f64 * c.cell_km2,
            "drainage_km2": river.drainage_km2,
            "length_km": river.length_m / 1000.0,
            "cells": river.cells,
            "parent": parent,
        }));
    }
    list.extend(dry);
    json!({
        "schema_version": 1,
        "kind": "water_bodies",
        "cells": "[column, row] from the north-west corner; a lake's exit_cell is the first cell \
            its routed flow enters outside it",
        "bodies": list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 x 5 world: a lake cell at (1, 1) drains into a river down column 2 to the sea row;
    /// (2, 2) is its exit, at the lake level.
    struct Scene {
        ground: Vec<f32>,
        dir: Vec<u8>,
        flags: Vec<u8>,
        body: Vec<u32>,
        surface: Vec<f32>,
        basins: Vec<Basin>,
        rivers: Vec<River>,
    }

    const W: usize = 5;
    type Mutation = fn(&mut Scene);
    fn at(y: usize, x: usize) -> usize {
        y * W + x
    }

    fn scene() -> Scene {
        let n = W * W;
        let mut s = Scene {
            ground: vec![10.0; n],
            dir: vec![2; n], // all drain south
            flags: vec![0; n],
            body: vec![0; n],
            surface: vec![f32::NAN; n],
            basins: vec![Basin::kept(Kind::Lake, 5.0, at(2, 2) as u32)],
            rivers: vec![River {
                mouth: at(3, 2) as u32,
                end: End::Sea,
                cells: 2,
                length_m: 2.0,
                drainage_km2: 20.0,
            }],
        };
        for x in 0..W {
            s.ground[at(4, x)] = -1.0;
            s.flags[at(4, x)] = SEA;
            s.dir[at(4, x)] = 0;
            s.surface[at(4, x)] = 0.0;
        }
        let lake = at(1, 1);
        (s.ground[lake], s.body[lake], s.surface[lake], s.dir[lake]) = (3.0, 1, 5.0, 8);
        for (y, g, w) in [(2, 4.0, 5.0), (3, 2.5, 3.0)] {
            let i = at(y, 2);
            (s.ground[i], s.surface[i], s.body[i]) = (g, w, 1);
            s.flags[i] |= RIVER;
        }
        s
    }

    fn run(s: &Scene) -> Map<String, Value> {
        let acc = vec![1.0; s.ground.len()];
        let c = Context {
            dims: Dims { w: W, h: W },
            ground: &s.ground,
            dir: &s.dir,
            acc: &acc,
            flags: &s.flags,
            body: &s.body,
            surface: &s.surface,
            basins: &s.basins,
            rivers: &s.rivers,
            cell_km2: 1.0,
            spacing_m: 1000.0,
        };
        checks(&c, 0)
    }

    fn failed(s: &Scene) -> Vec<String> {
        run(s)
            .into_iter()
            .filter(|(_, v)| v.as_u64() != Some(0))
            .map(|(k, _)| k)
            .collect()
    }

    #[test]
    fn every_check_can_fail() {
        assert!(failed(&scene()).is_empty(), "{:?}", failed(&scene()));
        let broken: [(&str, Mutation); 13] = [
            ("guard_land_cells_not_draining_to_an_outlet", |s| {
                s.dir[at(2, 1)] = 0
            }),
            ("guard_lakes_with_several_levels", |s| {
                let i = at(1, 2);
                (s.ground[i], s.body[i], s.surface[i], s.dir[i]) = (3.0, 1, 5.5, 2);
                s.flags[i] = 0;
            }),
            ("land_cells_draining_uphill", |s| s.ground[at(3, 1)] = 11.0),
            ("lakes_in_several_pieces", |s| {
                let i = at(1, 4);
                (s.ground[i], s.body[i], s.surface[i]) = (3.0, 1, 5.0);
            }),
            ("lake_cells_not_below_level", |s| s.ground[at(1, 1)] = 6.0),
            ("lake_exits_off_level", |s| s.surface[at(2, 2)] = 5.5),
            ("river_cells_below_adjacent_lake", |s| {
                s.surface[at(2, 2)] = 4.5
            }),
            ("wet_cells_with_surface_not_above_ground", |s| {
                s.surface[at(2, 2)] = 3.9
            }),
            ("river_cells_rising_downstream", |s| {
                s.surface[at(3, 2)] = 5.1
            }),
            ("rivers_ending_on_dry_land", |s| s.rivers[0].end = End::Land),
            ("dry_cells_below_adjacent_lake", |s| {
                s.ground[at(0, 1)] = 4.0
            }),
            ("dry_cells_below_adjacent_sea", |s| {
                s.ground[at(3, 0)] = -0.5
            }),
            ("dry_cells_below_adjacent_river", |s| {
                s.ground[at(3, 3)] = 2.0
            }),
        ];
        for (name, mutate) in broken {
            let mut s = scene();
            mutate(&mut s);
            assert!(
                failed(&s).contains(&name.to_string()),
                "{name}: {:?}",
                failed(&s)
            );
        }
    }
}
