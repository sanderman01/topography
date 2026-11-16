//! Ice as a phase of standing water (owner ruling 2026-11-11, I1): a lake mostly on cold
//! cells (snow/ice or glacier), or mostly ringed by them, is kind `ice` in bodies.json. It
//! keeps its cells in the body raster and its flat surface at the lake's level, so the game
//! gets an ice sheet, not a dry bed. The ring catches lakes the snow layer leaves bare (it
//! has no snow on coarser-level water). Routing is unchanged: its water still leaves by the
//! lake's exit, and the ground is kept as is.

use super::geom::Rect;
use super::layers::Layer;
use super::store::for_tiles;
use crate::acquisition::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Per lake: (cells, cold cells, shore ring cells, cold ring cells).
type Tally = BTreeMap<u32, [u64; 4]>;

/// A lake's cells: bounding box [x0, y0, x1, y1] (inclusive), count and coordinate sums.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Extent {
    pub bbox: [u64; 4],
    pub cells: u64,
    pub sum: [u64; 2],
}

impl Extent {
    fn add(&mut self, o: &Extent) {
        let b = &mut self.bbox;
        *b = [
            b[0].min(o.bbox[0]),
            b[1].min(o.bbox[1]),
            b[2].max(o.bbox[2]),
            b[3].max(o.bbox[3]),
        ];
        self.cells += o.cells;
        self.sum = [self.sum[0] + o.sum[0], self.sum[1] + o.sum[1]];
    }
    /// The centroid cell's centre (x, y), in cells.
    pub fn centroid(&self) -> (f64, f64) {
        let n = self.cells.max(1) as f64;
        (self.sum[0] as f64 / n + 0.5, self.sum[1] as f64 / n + 0.5)
    }
}

/// The ids of lakes in `bodies` whose cells in `body`, or whose shore ring (the cells
/// 8-adjacent to the lake but not in it), are more than `share` cold. `cold` gives the cold
/// cells of a window. Also each lake's extent (one pass over the body raster).
pub(crate) fn frozen(
    tiles: &[Rect],
    threads: usize,
    body: &Layer<u32>,
    cold: impl Fn(Rect) -> Result<Vec<bool>> + Sync,
    bodies: &Value,
    share: f64,
) -> Result<(BTreeSet<u32>, BTreeMap<u32, Extent>)> {
    let lakes: BTreeSet<u32> = listed(bodies, "lake").collect();
    let world = body.world();
    let per_tile = for_tiles(tiles, threads, "ice", |_, core| {
        let halo = core.grow(1, world);
        let (b, c) = (body.read(halo)?, cold(core)?);
        let mut n = Tally::new();
        let mut ext: BTreeMap<u32, Extent> = BTreeMap::new();
        let mut near = Vec::with_capacity(8);
        for (k, &cold) in c.iter().enumerate() {
            let (x, y) = core.world_xy(k);
            let id = b[halo.local(x, y).expect("core in halo")];
            if lakes.contains(&id) {
                let e = n.entry(id).or_default();
                e[0] += 1;
                e[1] += u64::from(cold);
                let one = Extent {
                    bbox: [x, y, x, y],
                    cells: 1,
                    sum: [x, y],
                };
                ext.entry(id).and_modify(|e| e.add(&one)).or_insert(one);
            }
            // Each ring cell counts once per lake it borders.
            near.clear();
            for d in 0..8 {
                if let Some((nx, ny)) = world.neighbour(x, y, d) {
                    let other = b[halo.local(nx, ny).expect("neighbour in halo")];
                    if other != id && lakes.contains(&other) && !near.contains(&other) {
                        near.push(other);
                    }
                }
            }
            for &l in &near {
                let e = n.entry(l).or_default();
                e[2] += 1;
                e[3] += u64::from(cold);
            }
        }
        Ok((n, ext))
    })?;
    let mut n = Tally::new();
    let mut extents: BTreeMap<u32, Extent> = BTreeMap::new();
    for (t, ext) in per_tile {
        for (id, e) in ext {
            extents.entry(id).and_modify(|a| a.add(&e)).or_insert(e);
        }
        for (id, v) in t {
            let e = n.entry(id).or_default();
            for (a, b) in e.iter_mut().zip(v) {
                *a += b;
            }
        }
    }
    let over = |part: u64, all: u64| all > 0 && part as f64 > share * all as f64;
    let ice = n
        .into_iter()
        .filter(|&(_, [a, ac, r, rc])| over(ac, a) || over(rc, r))
        .map(|(id, _)| id)
        .collect();
    Ok((ice, extents))
}

/// Relabel the frozen lakes `ice` (their other fields kept).
pub(crate) fn relabel(bodies: &mut Value, ice: &BTreeSet<u32>) {
    for b in bodies["bodies"].as_array_mut().into_iter().flatten() {
        let id = b["id"].as_u64().and_then(|v| u32::try_from(v).ok());
        if b["kind"] == "lake" && id.is_some_and(|id| ice.contains(&id)) {
            b["kind"] = "ice".into();
        }
    }
}

fn listed<'a>(bodies: &'a Value, kind: &'a str) -> impl Iterator<Item = u32> + 'a {
    bodies["bodies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(move |b| b["kind"] == kind)
        .filter_map(|b| b["id"].as_u64().and_then(|v| u32::try_from(v).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::{World, tiles};
    use serde_json::json;

    /// A bare lake (2 x 2) in a snowfield is ice by its shore ring; split over four tiles,
    /// each ring cell counts once.
    #[test]
    fn a_bare_lake_ringed_by_snow_is_ice_at_any_tiling() {
        let world = World { w: 6, h: 6 };
        let inside = |k: usize| (2..4).contains(&(k % 6)) && (2..4).contains(&(k / 6));
        let ids: Vec<u32> = (0..36).map(|k| if inside(k) { 7 } else { 0 }).collect();
        let bodies = json!({"bodies": [{"id": 7, "kind": "lake"}]});
        let body = Layer::memory(world, ids);
        // The 12-cell ring: 7 cold (> 0.5) and 6 cold (not).
        let ring: Vec<usize> = (0..36)
            .filter(|&k| !inside(k) && (1..5).contains(&(k % 6)) && (1..5).contains(&(k / 6)))
            .collect();
        assert_eq!(ring.len(), 12);
        for (n, want) in [(7, true), (6, false)] {
            let cold: Vec<bool> = (0..36).map(|k| ring[..n].contains(&k)).collect();
            for size in [6, 3, 2] {
                let got = frozen(
                    &tiles(world, size),
                    2,
                    &body,
                    mask(world, cold.clone()),
                    &bodies,
                    0.5,
                )
                .expect("frozen")
                .0;
                assert_eq!(got.contains(&7), want, "{n} cold ring cells, tile {size}");
            }
        }
    }

    /// Cold cells of a window from a whole-world mask.
    fn mask(world: World, cold: Vec<bool>) -> impl Fn(Rect) -> Result<Vec<bool>> + Sync {
        move |r: Rect| {
            Ok((0..r.cells())
                .map(|k| {
                    let (x, y) = r.world_xy(k);
                    cold[world.index(x, y) as usize]
                })
                .collect())
        }
    }

    /// A 4 x 4 world with lake 2 on the left half (cold) and lake 3 on the right (rock).
    #[test]
    fn a_lake_mostly_on_snow_or_glacier_becomes_ice() {
        let world = World { w: 4, h: 4 };
        let ids: Vec<u32> = (0..16).map(|k| if k % 4 < 2 { 2 } else { 3 }).collect();
        // Lake 2: 3 of its 8 cells are not cold; lake 3: 2 of 8 are. Each shore ring (the
        // other lake's column next to it) is 2 of 4 cold.
        let cold: Vec<bool> = (0..16)
            .map(|k| match (k % 4, k / 4) {
                (0 | 1, 0..=2) => k != 9,
                (2, 0..2) => true,
                _ => false,
            })
            .collect();
        let body = Layer::memory(world, ids);
        let mut bodies = json!({"bodies": [
            {"id": 1, "kind": "sea"}, {"id": 2, "kind": "lake"}, {"id": 3, "kind": "lake"},
            {"id": 4, "kind": "river"}]});
        let all = tiles(world, 2);
        let cold = mask(world, cold);
        let (ice, ext) = frozen(&all, 2, &body, &cold, &bodies, 0.5).expect("frozen");
        assert_eq!(ice, BTreeSet::from([2]));
        assert_eq!(ext[&2].bbox, [0, 0, 1, 3]);
        assert_eq!((ext[&3].cells, ext[&3].centroid()), (8, (3.0, 2.0)));
        assert!(
            frozen(&all, 2, &body, &cold, &bodies, 0.7)
                .expect("frozen")
                .0
                .is_empty()
        );
        relabel(&mut bodies, &ice);
        let kinds: Vec<&str> = bodies["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["sea", "ice", "lake", "river"]);
    }
}
