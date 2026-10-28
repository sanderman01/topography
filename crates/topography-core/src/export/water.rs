//! Water bodies, level-0 terrain rows, the coupled terrain and water reducer, and the
//! water-surface extension past the shore.

use super::pyramid::{Reduce, W, mean_at, taps};
use super::tiles::Checks;
use crate::acquisition::{AcquisitionError, Result, digest, ensure, io_context, read_small};
use serde_json::Value;
use std::{collections::HashMap, fs, io::Read, path::Path};

/// Texels past the shore that carry the nearest water's surface.
pub(super) const REACH: usize = 4;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Sea,
    Lake,
    River,
}

/// Water bodies by export id (index 0 unused): sea first, then lakes and rivers by world id.
pub(super) struct Bodies {
    // World ids and parents are read by the export description.
    #[allow(dead_code)]
    pub world_id: Vec<u32>,
    pub kind: Vec<Kind>,
    /// Lakes and sea; NaN for rivers.
    pub level_m: Vec<f32>,
    /// Export id of the body the water flows into, 0 for none or a dry basin.
    #[allow(dead_code)]
    pub parent: Vec<u16>,
    by_world: HashMap<u32, u16>,
}

impl Bodies {
    /// Reads `bodies.json`, checked against the world manifest; dry basins are left out.
    pub fn load(world: &Path) -> Result<Self> {
        const LIMIT: u64 = 256 << 20;
        let manifest: Value = serde_json::from_slice(&read_small(&world.join("manifest.json"))?)?;
        let path = world.join("bodies.json");
        let file = io_context("open bodies.json", fs::File::open(&path))?;
        let mut bytes = Vec::new();
        io_context(
            "read bodies.json",
            file.take(LIMIT + 1).read_to_end(&mut bytes),
        )?;
        ensure(bytes.len() as u64 <= LIMIT, "bodies.json exceeds 256 MiB")?;
        ensure(
            manifest["layers"]["bodies.json"].as_str() == Some(digest(&bytes).as_str()),
            "bodies.json does not match the world manifest",
        )?;
        let list: Value = serde_json::from_slice(&bytes)?;
        let entries = list["bodies"]
            .as_array()
            .ok_or_else(|| AcquisitionError::Invalid("bodies.json has no bodies".into()))?;
        // The sea is world id 1 even when a world without coast leaves it out.
        let mut sea = (1u32, 0.0f32, 0u32);
        let mut water = Vec::new();
        for e in entries {
            let id = e["id"].as_u64().and_then(|v| u32::try_from(v).ok());
            let id = id.ok_or_else(|| AcquisitionError::Invalid("body without an id".into()))?;
            let level = e["level_m"].as_f64().map_or(f32::NAN, |v| v as f32);
            let parent = e["parent"].as_u64().unwrap_or(0) as u32;
            match e["kind"].as_str() {
                Some("sea") => {
                    ensure(level.is_finite(), "sea without level_m")?;
                    sea = (id, level, parent);
                }
                Some("lake") => {
                    ensure(level.is_finite(), format!("lake {id} without level_m"))?;
                    water.push((id, Kind::Lake, level, parent));
                }
                Some("river") => water.push((id, Kind::River, f32::NAN, parent)),
                Some("dry") => {}
                other => {
                    return Err(AcquisitionError::Invalid(format!(
                        "body {id} has unknown kind {other:?}"
                    )));
                }
            }
        }
        water.sort_by_key(|b| b.0);
        ensure(
            water.len() < usize::from(u16::MAX),
            "more than 65,535 water bodies",
        )?;
        let mut all = vec![
            (0, Kind::Sea, f32::NAN, 0),
            (sea.0, Kind::Sea, sea.1, sea.2),
        ];
        all.extend(water);
        let mut by_world = HashMap::with_capacity(all.len());
        for (i, b) in all.iter().enumerate().skip(1) {
            ensure(
                by_world.insert(b.0, i as u16).is_none(),
                format!("body {} is listed twice", b.0),
            )?;
        }
        let parent = all
            .iter()
            .map(|b| by_world.get(&b.3).copied().unwrap_or(0))
            .collect();
        Ok(Self {
            world_id: all.iter().map(|b| b.0).collect(),
            kind: all.iter().map(|b| b.1).collect(),
            level_m: all.iter().map(|b| b.2).collect(),
            parent,
            by_world,
        })
    }

    pub fn export_id(&self, world_id: u32) -> Option<u16> {
        self.by_world.get(&world_id).copied()
    }

    pub fn len(&self) -> usize {
        self.kind.len()
    }
}

/// One row of ground, water coverage, surface (NaN where dry) and export body id.
#[derive(Clone)]
pub(super) struct TerrainRow {
    pub ground: Vec<f32>,
    pub coverage: Vec<f32>,
    pub surface: Vec<f32>,
    pub body: Vec<u16>,
}

/// Area mean of ground and coverage; body sampled, surface per body (see `reduce`).
pub(super) struct Terrain<'a> {
    pub bodies: &'a Bodies,
}

impl Reduce for Terrain<'_> {
    type Row = TerrainRow;
    fn reduce(&self, rows: [&TerrainRow; 3]) -> TerrainRow {
        let [a, b, c] = rows;
        debug_assert!(a.ground.len() == b.ground.len() && c.ground.len() == b.ground.len());
        let n = b.ground.len() / 2;
        fn ground(r: &TerrainRow) -> &[f32] {
            &r.ground
        }
        fn coverage(r: &TerrainRow) -> &[f32] {
            &r.coverage
        }
        let mut out = TerrainRow {
            ground: (0..n).map(|i| mean_at(rows.map(ground), i)).collect(),
            coverage: (0..n).map(|i| mean_at(rows.map(coverage), i)).collect(),
            surface: Vec::with_capacity(n),
            body: Vec::with_capacity(n),
        };
        for i in 0..n {
            let t = taps(i);
            let samples = move || {
                rows.into_iter().zip(W).flat_map(move |(r, wy)| {
                    t.into_iter().zip(W).map(move |(x, wx)| (r, x, wy * wx))
                })
            };
            let body = if b.coverage[2 * i] > 0.0 {
                b.body[2 * i]
            } else {
                // The heaviest wet body of the footprint, ties to the lowest id.
                let (mut wet, mut k) = ([(0u16, 0f32); 9], 0);
                for (r, x, w) in samples().filter(|(r, x, _)| r.coverage[*x] > 0.0) {
                    match wet[..k].iter_mut().find(|e| e.0 == r.body[x]) {
                        Some(e) => e.1 += w,
                        None => {
                            wet[k] = (r.body[x], w);
                            k += 1;
                        }
                    }
                }
                wet[..k]
                    .iter()
                    .fold((0u16, 0f32), |best, &e| {
                        if e.1 > best.1 || (e.1 == best.1 && e.0 < best.0) {
                            e
                        } else {
                            best
                        }
                    })
                    .0
            };
            let id = usize::from(body);
            let surface = match self.bodies.kind[id] {
                _ if body == 0 => f32::NAN,
                Kind::Sea | Kind::Lake => self.bodies.level_m[id],
                Kind::River => {
                    let (mut sum, mut weight) = (0.0, 0.0);
                    for (r, x, w) in samples().filter(|(r, x, _)| r.body[*x] == body) {
                        sum += w * r.surface[x];
                        weight += w;
                    }
                    sum / weight
                }
            };
            out.body.push(body);
            out.surface.push(surface);
        }
        out
    }
}

fn note(checks: &mut Checks, check: &str, x: usize, y: usize) {
    if checks.examples.len() < 3 {
        checks.example(format!("{check} at ({x}, {y})"));
    }
}

/// Level-0 row; counts world inconsistencies into `checks`. Inconsistent cells are dry.
/// `code` is the height encoding, used to compare lake and sea surfaces with their level.
#[allow(clippy::too_many_arguments)]
pub(super) fn level0(
    ground: &[f32],
    surface: &[f32],
    body: &[u32],
    bodies: &Bodies,
    code: &dyn Fn(f32) -> u16,
    checks: &mut Checks,
    y: usize,
) -> Result<TerrainRow> {
    let n = ground.len();
    debug_assert!(surface.len() == n && body.len() == n);
    let mut row = TerrainRow {
        ground: ground.to_vec(),
        coverage: vec![0.0; n],
        surface: vec![f32::NAN; n],
        body: vec![0; n],
    };
    for x in 0..n {
        let (g, s, b) = (ground[x], surface[x], body[x]);
        if b == 0 {
            if s.is_finite() {
                checks.surface_without_body += 1;
                note(checks, "surface_without_body", x, y);
            }
            continue;
        }
        let id = bodies.export_id(b).ok_or_else(|| {
            AcquisitionError::Invalid(format!(
                "water body {b} at ({x}, {y}) is not a water body of bodies.json"
            ))
        })?;
        let i = usize::from(id);
        let s = match bodies.kind[i] {
            Kind::Sea | Kind::Lake if s.is_finite() => {
                let level = bodies.level_m[i];
                if code(s) != code(level) {
                    checks.lake_levels_off += 1;
                    note(checks, "lake_levels_off", x, y);
                }
                level
            }
            _ => s,
        };
        if !s.is_finite() {
            checks.wet_without_surface += 1;
            note(checks, "wet_without_surface", x, y);
        } else if s <= g {
            checks.wet_not_above_ground += 1;
            note(checks, "wet_not_above_ground", x, y);
        } else {
            row.coverage[x] = 1.0;
            row.surface[x] = s;
            row.body[x] = id;
        }
    }
    Ok(row)
}

/// Output surface for a window of rows (rows y0..y0+n): wet texels keep theirs; dry texels
/// take the nearest wet texel's within `reach` (Chebyshev rings, then smallest squared
/// distance, then scan order), else their ground.
pub(super) fn extended_surface(rows: &[&TerrainRow], reach: usize) -> Vec<Vec<f32>> {
    let n = rows.len();
    let w = rows.first().map_or(0, |r| r.body.len());
    // Wet texels per column within `reach` rows, slid down the window.
    let mut column = vec![0u32; w];
    let add = |column: &mut [u32], y: usize, up: bool| {
        for (c, &b) in column.iter_mut().zip(&rows[y].body) {
            if b > 0 {
                if up { *c += 1 } else { *c -= 1 }
            }
        }
    };
    for y in 0..reach.min(n) {
        add(&mut column, y, true);
    }
    let mut prefix = vec![0u32; w + 1];
    let mut out = Vec::with_capacity(n);
    for y in 0..n {
        if y + reach < n {
            add(&mut column, y + reach, true);
        }
        if y > reach {
            add(&mut column, y - reach - 1, false);
        }
        for x in 0..w {
            prefix[x + 1] = prefix[x] + column[x];
        }
        let row = rows[y];
        let surface = (0..w)
            .map(|x| {
                if row.body[x] > 0 {
                    return row.surface[x];
                }
                if prefix[(x + reach + 1).min(w)] == prefix[x.saturating_sub(reach)] {
                    return row.ground[x];
                }
                nearest(rows, x, y, reach).unwrap_or(row.ground[x])
            })
            .collect();
        out.push(surface);
    }
    out
}

/// Surface of the nearest wet texel within `reach` rings of (x, y).
fn nearest(rows: &[&TerrainRow], x: usize, y: usize, reach: usize) -> Option<f32> {
    let (w, n) = (rows[0].body.len() as isize, rows.len() as isize);
    let (x, y) = (x as isize, y as isize);
    for d in 1..=reach as isize {
        let mut best: Option<(isize, f32)> = None;
        for dy in -d..=d {
            let yy = y + dy;
            if !(0..n).contains(&yy) {
                continue;
            }
            let r = rows[yy as usize];
            let step = if dy.abs() == d { 1 } else { 2 * d as usize };
            for dx in (-d..=d).step_by(step) {
                let xx = x + dx;
                if !(0..w).contains(&xx) || r.body[xx as usize] == 0 {
                    continue;
                }
                let d2 = dx * dx + dy * dy;
                if best.is_none_or(|(b, _)| d2 < b) {
                    best = Some((d2, r.surface[xx as usize]));
                }
            }
        }
        if let Some((_, s)) = best {
            return Some(s);
        }
    }
    None
}

/// Per export id at level 0: texel count and [x0, y0, x1, y1] sample bounds (x1 and y1
/// exclusive; empty bodies keep [MAX, MAX, 0, 0]).
// Read by the export description.
#[allow(dead_code)]
pub(super) struct WaterStats {
    pub cells: Vec<u64>,
    pub bounds: Vec<[usize; 4]>,
}

impl WaterStats {
    pub fn new(bodies: &Bodies) -> Self {
        Self {
            cells: vec![0; bodies.len()],
            bounds: vec![[usize::MAX, usize::MAX, 0, 0]; bodies.len()],
        }
    }

    pub fn add(&mut self, row: &TerrainRow, y: usize) {
        for (x, &b) in row.body.iter().enumerate().filter(|(_, b)| **b > 0) {
            let i = usize::from(b);
            self.cells[i] += 1;
            let [x0, y0, x1, y1] = &mut self.bounds[i];
            *x0 = (*x0).min(x);
            *y0 = (*y0).min(y);
            *x1 = (*x1).max(x + 1);
            *y1 = (*y1).max(y + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Layout, heightmap, testworld, tile_path, view::read_png};
    use super::*;
    use std::collections::BTreeSet;

    /// 16-bit codes of a tile with its apron stripped, row-major.
    fn codes(out: &Path, layer: &str, level: u32, tx: usize, ty: usize) -> (usize, Vec<u16>) {
        let (w, h, px) = read_png(&out.join(tile_path(layer, level, tx, ty))).unwrap();
        let a = if layer == "height" { 0 } else { 4 };
        let side = w - 2 * a;
        let v = (a..h - a)
            .flat_map(|r| (a..w - a).map(move |c| r * w + c))
            .map(|i| u16::from_be_bytes([px[2 * i], px[2 * i + 1]]))
            .collect();
        (side, v)
    }

    /// Surface codes where the body is the lake (export id 2) at `level`.
    fn lake_codes(out: &Path, level: u32) -> BTreeSet<u16> {
        let mut set = BTreeSet::new();
        for entry in fs::read_dir(out.join(format!("water-body/L{level}"))).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let xy: Vec<usize> = name
                .trim_end_matches(".png")
                .split('_')
                .skip(2)
                .map(|v| v.parse().unwrap())
                .collect();
            let (_, body) = codes(out, "water-body", level, xy[0], xy[1]);
            let (_, surface) = codes(out, "water-surface", level, xy[0], xy[1]);
            set.extend(
                body.iter()
                    .zip(&surface)
                    .filter(|(b, _)| **b == 2)
                    .map(|(_, s)| *s),
            );
        }
        set
    }

    #[test]
    fn lake_level_is_identical_at_every_level() {
        // A lake (world id 2) over an uneven bed, a river (id 10) beside it.
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, y| {
            let lake = (64..128).contains(&x) && (64..128).contains(&y);
            let river = x == 129 && y < 200;
            let g = if lake { 5.0 + (x % 7) as f32 } else { 20.0 };
            match (lake, river) {
                (true, _) => (g, f32::NAN, 2),
                (_, true) => (18.0, 19.0, 10),
                _ => (g, f32::NAN, 0),
            }
        });
        let out = dir.path().join("x");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        for level in 0..=4 {
            assert_eq!(lake_codes(&out, level).len(), 1, "level {level}");
        }
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let (h_min, step) = (
            m["encoding"]["h_min_m"].as_f64().unwrap(),
            m["encoding"]["step_m"].as_f64().unwrap(),
        );
        let code = *lake_codes(&out, 0).first().unwrap();
        assert!((h_min + f64::from(code) * step - 12.0).abs() <= step);
        assert!(m["water_files"].is_string());
        assert_eq!(m["layers"]["water-surface"]["scale"].as_f64(), Some(step));
    }

    #[test]
    fn coverage_keeps_a_thin_stream_share_and_body_follows_coverage() {
        // A 1 m stream on column 33: L1 texels 16 and 17 hold a quarter of it.
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, _| {
            if x == 33 {
                (10.0, 11.0, 10)
            } else {
                (20.0, f32::NAN, 0)
            }
        });
        let out = dir.path().join("x");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        // L1 texel 16 is column 0 of tile 1 (plus the apron of 4).
        let (w, _, c) = read_png(&out.join(tile_path("water-coverage", 1, 1, 2))).unwrap();
        let (_, _, b) = read_png(&out.join(tile_path("water-body", 1, 1, 2))).unwrap();
        let row = 9 * w;
        let body = |i: usize| u16::from_be_bytes([b[2 * (row + i)], b[2 * (row + i) + 1]]);
        assert_eq!(&c[row + 3..row + 7], [0, 64, 64, 0]);
        assert_eq!([body(3), body(4), body(5), body(6)], [0, 2, 2, 0]);
        // Coverage > 0 exactly where body > 0, in every written tile.
        for level in 0..=4 {
            for entry in fs::read_dir(out.join(format!("water-coverage/L{level}"))).unwrap() {
                let p = entry.unwrap().path();
                let (_, _, c) = read_png(&p).unwrap();
                let q = p.to_str().unwrap().replace("water-coverage", "water-body");
                let (_, _, b) = read_png(Path::new(&q)).unwrap();
                for (i, &v) in c.iter().enumerate() {
                    assert_eq!(v > 0, b[2 * i] != 0 || b[2 * i + 1] != 0, "{q}");
                }
            }
        }
    }

    #[test]
    fn surface_extends_four_texels_then_holds_ground() {
        // A lake west of column 100 (level 6 m), dry ground at 20 m east of it.
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, _| {
            if x < 100 {
                (5.0, f32::NAN, 2)
            } else {
                (20.0, f32::NAN, 0)
            }
        });
        let out = dir.path().join("x");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        // Columns 96..113 of L0 are tile 6; row 5.
        let (side, s) = codes(&out, "water-surface", 0, 6, 3);
        let (_, g) = codes(&out, "height", 0, 6, 3);
        let at = |v: &[u16], x: usize| v[5 * side + x - 96];
        let water = at(&s, 99);
        assert_ne!(water, at(&g, 104));
        for x in 100..=103 {
            assert_eq!(at(&s, x), water, "column {x}");
        }
        assert_eq!(at(&s, 104), at(&g, 104));
        assert_eq!(at(&s, 110), at(&g, 110));
    }

    #[test]
    fn water_files_only_where_water() {
        // A lake in L0 tile (0, 0) whose east end lies in the apron of tile (1, 0).
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, y| {
            if (4..=13).contains(&x) && (4..=8).contains(&y) {
                (5.0, f32::NAN, 2)
            } else {
                (20.0, f32::NAN, 0)
            }
        });
        let out = dir.path().join("x");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        for layer in ["water-surface", "water-coverage", "water-body"] {
            let mut names: Vec<String> = fs::read_dir(out.join(format!("{layer}/L0")))
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            assert_eq!(
                names,
                [format!("{layer}_L0_0_0.png"), format!("{layer}_L0_1_0.png")]
            );
        }
        assert_eq!(fs::read_dir(out.join("height/L0")).unwrap().count(), 256);
    }

    #[test]
    fn extension_prefers_the_inner_ring_then_distance() {
        let row = |body: Vec<u16>, surface: Vec<f32>| TerrainRow {
            ground: vec![0.0; body.len()],
            coverage: body.iter().map(|&b| f32::from(b > 0)).collect(),
            surface,
            body,
        };
        let a = row(
            vec![0, 0, 0, 0, 1],
            vec![f32::NAN, f32::NAN, f32::NAN, f32::NAN, 3.0],
        );
        let b = row(
            vec![0, 0, 0, 2, 0],
            vec![f32::NAN, f32::NAN, f32::NAN, 7.0, f32::NAN],
        );
        let s = extended_surface(&[&a, &b], 4);
        // (3, 0) has (4, 0) and (3, 1) at distance 1: scan order takes (4, 0).
        assert_eq!(s[0][3], 3.0);
        // (2, 0): ring 1 holds (3, 1); (4, 0) is ring 2.
        assert_eq!(s[0][2], 7.0);
        // (0, 0): ring 3 holds (3, 1) at distance² 10, ring 4 is not reached.
        assert_eq!(s[0][0], 7.0);
        let s = extended_surface(&[&a, &b], 2);
        assert_eq!(s[0][0], 0.0);
    }
}
