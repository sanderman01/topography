//! Ice as a phase of standing water (owner ruling 2026-11-11, I1): a lake mostly on glacier
//! cells is kind `ice` in bodies.json and leaves the water surface and body rasters. Routing
//! is unchanged: its meltwater still leaves by the lake's exit.

use super::geom::Rect;
use super::layers::{Classes, Layer};
use super::store::for_tiles;
use super::table::GLACIER;
use crate::acquisition::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The ids of lakes in `bodies` whose cells in `body` are more than `share` glacier.
pub(crate) fn frozen(
    tiles: &[Rect],
    threads: usize,
    body: &Layer<u32>,
    classes: &Classes,
    bodies: &Value,
    share: f64,
) -> Result<BTreeSet<u32>> {
    let lakes: BTreeSet<u32> = listed(bodies, "lake").collect();
    let per_tile = for_tiles(tiles, threads, "ice", |_, core| {
        let (b, c) = (body.read(core)?, classes.read(core)?);
        let mut n: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
        for (&id, &k) in b.iter().zip(&c) {
            if lakes.contains(&id) {
                let e = n.entry(id).or_default();
                e.0 += 1;
                e.1 += u64::from(k == GLACIER);
            }
        }
        Ok(n)
    })?;
    let mut n: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    for t in per_tile {
        for (id, (a, g)) in t {
            let e = n.entry(id).or_default();
            (e.0, e.1) = (e.0 + a, e.1 + g);
        }
    }
    Ok(n.into_iter()
        .filter(|&(_, (a, g))| g as f64 > share * a as f64)
        .map(|(id, _)| id)
        .collect())
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

/// Clear ice cells from a water surface (NaN) and body raster (0), in place.
pub(crate) fn clear(
    tiles: &[Rect],
    threads: usize,
    surface: &Layer<f32>,
    body: &Layer<u32>,
    ice: &BTreeSet<u32>,
) -> Result<()> {
    for_tiles(tiles, threads, "ice clear", |_, core| {
        let mut b = body.read(core)?;
        if !b.iter().any(|id| ice.contains(id)) {
            return Ok(());
        }
        let mut s = surface.read(core)?;
        for (id, z) in b.iter_mut().zip(s.iter_mut()) {
            if ice.contains(id) {
                (*id, *z) = (0, f32::NAN);
            }
        }
        body.write(core, &b)?;
        surface.write(core, &s)
    })?;
    Ok(())
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

    /// A 4 x 4 world with lake 2 on the left half (glacier) and lake 3 on the right (rock).
    #[test]
    fn a_lake_mostly_on_glacier_becomes_ice_and_leaves_the_water_rasters() {
        let world = World { w: 4, h: 4 };
        let ids: Vec<u32> = (0..16).map(|k| if k % 4 < 2 { 2 } else { 3 }).collect();
        // Lake 2: 3 of its 8 cells are not glacier; lake 3: 2 of 8 are.
        let cls: Vec<u8> = (0..16)
            .map(|k| match (k % 4, k / 4) {
                (0 | 1, 0..=2) if k != 9 => GLACIER,
                (2, 0..2) => GLACIER,
                _ => 11,
            })
            .collect();
        let classes = Classes::new(Layer::memory(world, cls), 1, world).expect("classes");
        let body = Layer::memory(world, ids);
        let surface = Layer::memory(world, vec![5.0f32; 16]);
        let mut bodies = json!({"bodies": [
            {"id": 1, "kind": "sea"}, {"id": 2, "kind": "lake"}, {"id": 3, "kind": "lake"},
            {"id": 4, "kind": "river"}]});
        let all = tiles(world, 2);
        let ice = frozen(&all, 2, &body, &classes, &bodies, 0.5).expect("frozen");
        assert_eq!(ice, BTreeSet::from([2]));
        assert!(
            frozen(&all, 2, &body, &classes, &bodies, 0.7)
                .expect("frozen")
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
        clear(&all, 2, &surface, &body, &ice).expect("clear");
        let (b, s) = (body.to_vec().unwrap(), surface.to_vec().unwrap());
        for k in 0..16 {
            if k % 4 < 2 {
                assert!(b[k] == 0 && s[k].is_nan(), "cell {k} is ice");
            } else {
                assert!(b[k] == 3 && s[k] == 5.0, "cell {k} stays water");
            }
        }
    }
}
