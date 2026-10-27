//! Bands: a grid whose top rows are the previous (northern) band's core (`--north`) takes
//! those rows as they are, so adjacent bands agree on the seam by construction, and the seam
//! check that compares two band outputs.

use super::flood::{Dims, FLOW_CENTRE};
use super::grid::{self, Grid};
use crate::acquisition::{Result, digest, ensure, read_small};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, path::Path};

/// Body kinds of a band's `bodies.json` by id.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BodyKind {
    Sea,
    Lake,
    River,
}

/// The northern band's outputs over this grid's top `rows` rows (its core).
pub(super) struct North {
    pub rows: usize,
    pub ground: Vec<f32>,
    pub surface: Vec<f32>,
    pub body: Vec<u32>,
    pub discharge: Vec<f32>,
    pub flow: Vec<u8>,
    pub kinds: HashMap<u32, BodyKind>,
    pub max_id: u32,
    pub digests: Map<String, Value>,
}

/// Row of `b`'s first row in `a`, if both are the same width, spacing and columns.
fn row_offset(a: &Grid, b: &Grid) -> Result<usize> {
    ensure(
        a.w == b.w && a.spacing_m == b.spacing_m && (a.nw_m[0] - b.nw_m[0]).abs() < 1e-6,
        "bands must share width, spacing and columns",
    )?;
    let rows = (a.nw_m[1] - b.nw_m[1]) / a.spacing_m;
    ensure(
        rows > -1e-6 && (rows - rows.round()).abs() < 1e-6,
        "the bands' rows are not aligned",
    )?;
    Ok(rows.round() as usize)
}

fn kinds(bodies: &Value) -> (HashMap<u32, BodyKind>, u32) {
    let mut kinds = HashMap::new();
    let mut max_id = 1;
    for b in bodies["bodies"].as_array().into_iter().flatten() {
        let id = b["id"].as_u64().unwrap_or(0) as u32;
        max_id = max_id.max(id);
        let kind = match b["kind"].as_str() {
            Some("sea") => BodyKind::Sea,
            Some("lake") => BodyKind::Lake,
            Some("river") => BodyKind::River,
            _ => continue,
        };
        kinds.insert(id, kind);
    }
    (kinds, max_id)
}

fn manifest(dir: &Path) -> Result<(Grid, Vec<u8>)> {
    let bytes = read_small(&dir.join("grid.json"))?;
    let m: Map<String, Value> = serde_json::from_slice(&bytes)?;
    Ok((grid::shape(&m)?, bytes))
}

pub(super) fn read_north(dir: &Path, grid: &Grid) -> Result<North> {
    let (a, bytes) = manifest(dir)?;
    let t = row_offset(&a, grid)?;
    ensure(
        t >= a.core_rows[0] && t < a.core_rows[1],
        "the grid's first row must lie in the --north band's core",
    )?;
    let rows = a.core_rows[1] - t;
    ensure(
        rows < grid.h && grid.core_rows[0] == rows,
        format!("the grid's core must start where the --north band's core ends (row {rows})"),
    )?;
    let size = [a.w, a.h];
    let window = |name: &str| (dir.join(name), [0, a.w], [t, t + rows]);
    let f32le = |b: [u8; 4]| f32::from_le_bytes(b);
    let read_f32 = |name: &str| {
        let (p, xs, ys) = window(name);
        grid::read_window(&p, size, xs, ys, f32le)
    };
    let (p, xs, ys) = window("water-body.u32");
    let body = grid::read_window(&p, size, xs, ys, u32::from_le_bytes)?;
    let (p, xs, ys) = window("flow.u8");
    let flow = grid::read_window(&p, size, xs, ys, |b: [u8; 1]| b[0])?;
    let bodies_bytes = read_small(&dir.join("bodies.json"))?;
    let (kinds, max_id) = kinds(&serde_json::from_slice(&bodies_bytes)?);
    let mut digests = Map::new();
    digests.insert("grid.json".into(), digest(&bytes).into());
    digests.insert("bodies.json".into(), digest(&bodies_bytes).into());
    Ok(North {
        rows,
        ground: read_f32("elevation.f32")?,
        surface: read_f32("water-surface.f32")?,
        body,
        discharge: read_f32("discharge.f32")?,
        flow,
        kinds,
        max_id,
        digests,
    })
}

/// Water surfaces equal within 1 mm (both dry counts as equal).
fn same_surface(a: f32, b: f32) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-3
}

/// Compare two adjacent band outputs (`north` then `south`). Rows of the northern core that
/// the southern grid holds must be identical (ground and surface within 1 mm, body ids
/// equal); across the seam (the last northern core row against the first southern core row)
/// no centreline may flow onto dry ground, no dry cell may sit more than 1 cm below adjacent
/// water (ground ahead of a river along its flow excepted, as in the band checks) and a lake
/// id keeps one level. Returns the report (JSON text) and whether it passed.
pub fn seam(north: &Path, south: &Path) -> Result<(String, bool)> {
    let (a, _) = manifest(north)?;
    let (b, _) = manifest(south)?;
    let t = row_offset(&a, &b)?;
    ensure(
        t < a.core_rows[1] && a.core_rows[1] - t == b.core_rows[0],
        "the southern core must start where the northern core ends",
    )?;
    let (w, rows) = (a.w, a.core_rows[1] - t);
    // Northern rows t.. up to its end; southern rows 0.. as many.
    let ra = [t, a.h];
    let rb = [0, (a.h - t).min(b.h)];
    // Ground, surface, body and flow rows.
    type Rows = (Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>);
    let load = |dir: &Path, g: &Grid, r: [usize; 2]| -> Result<Rows> {
        let size = [g.w, g.h];
        let f = |b: [u8; 4]| f32::from_le_bytes(b);
        Ok((
            grid::read_window(&dir.join("elevation.f32"), size, [0, w], r, f)?,
            grid::read_window(&dir.join("water-surface.f32"), size, [0, w], r, f)?,
            grid::read_window(
                &dir.join("water-body.u32"),
                size,
                [0, w],
                r,
                u32::from_le_bytes,
            )?,
            grid::read_window(&dir.join("flow.u8"), size, [0, w], r, |b: [u8; 1]| b[0])?,
        ))
    };
    let (ga, sa, ba, fa) = load(north, &a, ra)?;
    let (gb, sb, bb, _) = load(south, &b, rb)?;
    let overlap = rb[1];
    let (mut ground_diff, mut surface_diff, mut body_diff) = (0u64, 0u64, 0u64);
    for i in 0..rows * w {
        ground_diff += u64::from((ga[i] - gb[i]).abs() > 1e-3);
        surface_diff += u64::from(!same_surface(sa[i], sb[i]));
        body_diff += u64::from(ba[i] != bb[i]);
    }
    // Stitched seam: northern rows up to `rows`, southern from there.
    let pick = |i: usize| {
        if i / w < rows {
            (ga[i], sa[i], ba[i])
        } else {
            (gb[i], sb[i], bb[i])
        }
    };
    let dims = Dims { w, h: overlap };
    let (mut cut, mut steps, mut levels) = (0u64, 0u64, 0u64);
    let mut lake_level: HashMap<u32, f32> = HashMap::new();
    let (bk, _) = kinds(&serde_json::from_slice(&read_small(
        &south.join("bodies.json"),
    )?)?);
    for x in 0..w {
        let i = (rows - 1) * w + x;
        // A northern centreline flowing south across the seam must land on water.
        let d = fa[i] & 15;
        if fa[i] & FLOW_CENTRE != 0 && d != 0 {
            if let Some(r) = dims.neighbour(i, d as usize - 1).filter(|&r| r / w >= rows) {
                cut += u64::from(pick(r).2 == 0);
            }
        }
        for (_, j) in dims.neighbours(i).filter(|&(_, j)| j / w == rows) {
            for (p, q) in [(i, j), (j, i)] {
                let (gp, _, bp) = pick(p);
                let (_, sq, bq) = pick(q);
                if bp != 0 || bq == 0 || sq <= gp + 0.01 {
                    continue;
                }
                // Ahead of a river along its flow: the next cross-section's bank.
                let fq = if q / w < rows { fa[q] } else { 0 };
                let ahead = bk.get(&bq) == Some(&BodyKind::River)
                    && fq & 15 != 0
                    && dims.ahead(fq & 15, q, p);
                steps += u64::from(!ahead);
            }
        }
        for c in [i, (rows) * w + x] {
            let (_, s, id) = pick(c);
            if bk.get(&id) == Some(&BodyKind::Lake) {
                let l = *lake_level.entry(id).or_insert(s);
                levels += u64::from(!same_surface(l, s));
            }
        }
    }
    // Southern core rows the northern halo also covers: how often the halo's guess held.
    let (mut same, mut halo) = (0u64, 0u64);
    for i in rows * w..overlap * w {
        halo += 1;
        same += u64::from(ba[i] == bb[i] && same_surface(sa[i], sb[i]));
    }
    let checks = json!({
        "shared_core_ground_differs": ground_diff,
        "shared_core_surface_differs": surface_diff,
        "shared_core_body_differs": body_diff,
        "centrelines_cut_at_seam": cut,
        "dry_cells_below_water_across_seam": steps,
        "lakes_with_two_levels_at_seam": levels,
    });
    let passed = checks
        .as_object()
        .is_some_and(|m| m.values().all(|v| v.as_u64() == Some(0)));
    let report = json!({
        "kind": "hydrology_seam",
        "north": north.display().to_string(),
        "south": south.display().to_string(),
        "shared_core_rows": rows,
        "passed": passed,
        "checks": checks,
        "halo_rows_compared": overlap - rows,
        "halo_agreement_share": if halo > 0 { same as f64 / halo as f64 } else { 1.0 },
    });
    Ok((serde_json::to_string_pretty(&report)?, passed))
}
