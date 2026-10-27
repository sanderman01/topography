//! Bands: a grid whose top rows are the previous (northern) band's core (`--north`) takes
//! those rows as they are, so adjacent bands agree on the seam by construction, and the seam
//! check that compares two band outputs.

use super::flood::{CENTRE, Dims, FLOW_CENTRE, FLOW_DRY, RIVER, SEA};
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
/// no centreline may flow onto dry ground that is not a basin, no dry cell may sit more than
/// 1 cm below an adjacent lake or the sea, and a lake id keeps one level (river banks are the
/// southern band's own checks). Returns the report (JSON text) and whether it passed.
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
    let (gb, sb, bb, fb) = load(south, &b, rb)?;
    // Body kinds from both bands (a river may run only in the northern rows).
    let mut bk = HashMap::new();
    for dir in [north, south] {
        let (k, _) = kinds(&serde_json::from_slice(&read_small(
            &dir.join("bodies.json"),
        )?)?);
        bk.extend(k);
    }
    let mut report = compare(w, t, rows, (&ga, &sa, &ba, &fa), (&gb, &sb, &bb, &fb), &bk);
    if let Value::Object(m) = &mut report {
        m.insert("north".into(), json!(north.display().to_string()));
        m.insert("south".into(), json!(south.display().to_string()));
    }
    let passed = report["passed"].as_bool().unwrap_or(false);
    Ok((serde_json::to_string_pretty(&report)?, passed))
}

/// Ground, surface, body and flow rows of a band output.
type RowsRef<'a> = (&'a [f32], &'a [f32], &'a [u32], &'a [u8]);

/// The seam comparison over rows from the southern grid's first row (`t` in the northern
/// one): the first `rows` are the northern core, the rest the northern halo.
fn compare(
    w: usize,
    t: usize,
    rows: usize,
    (ga, sa, ba, fa): RowsRef,
    (gb, sb, bb, fb): RowsRef,
    bk: &HashMap<u32, BodyKind>,
) -> Value {
    let overlap = ga.len().min(gb.len()) / w;
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
    let mut step_at: Vec<[usize; 2]> = Vec::new();
    let mut river_steps = 0u64;
    let mut lake_level: HashMap<u32, f32> = HashMap::new();
    for x in 0..w {
        let i = (rows - 1) * w + x;
        // A northern centreline flowing south across the seam must land on water.
        let d = fa[i] & 15;
        if fa[i] & FLOW_CENTRE != 0 && d != 0 {
            if let Some(r) = dims.neighbour(i, d as usize - 1).filter(|&r| r / w >= rows) {
                // (Ending in a dry basin there is fine.)
                cut += u64::from(pick(r).2 == 0 && fb[r] & FLOW_DRY == 0);
            }
        }
        for (_, j) in dims.neighbours(i).filter(|&(_, j)| j / w == rows) {
            for (p, q) in [(i, j), (j, i)] {
                let (gp, _, bp) = pick(p);
                let (_, sq, bq) = pick(q);
                if bp != 0 || bq == 0 || sq <= gp + 0.01 {
                    continue;
                }
                // Lakes and the sea strictly. Beside rivers the southern band's own checks
                // decide (its grid holds both sides, with each channel cell's centreline);
                // here only cells not ahead of the wet cell's own flow are counted, as a figure.
                if bk.get(&bq) == Some(&BodyKind::River) {
                    let fq = if q / w < rows { fa[q] } else { fb[q] };
                    river_steps += u64::from(fq & 15 == 0 || !dims.ahead(fq & 15, q, p));
                    continue;
                }
                steps += 1;
                if step_at.len() < 5 {
                    step_at.push([p % w, t + p / w]);
                }
            }
        }
        for c in [i, rows * w + x] {
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
        "dry_cells_below_lake_or_sea_across_seam": steps,
        "lakes_with_two_levels_at_seam": levels,
    });
    let passed = checks
        .as_object()
        .is_some_and(|m| m.values().all(|v| v.as_u64() == Some(0)));
    json!({
        "kind": "hydrology_seam",
        "shared_core_rows": rows,
        "passed": passed,
        "checks": checks,
        "halo_rows_compared": overlap - rows,
        "dry_cells_below_water_at": step_at,
        "dry_cells_below_a_river_across_seam_not_ahead_of_its_own_flow": river_steps,
        "halo_agreement_share": if halo > 0 { same as f64 / halo as f64 } else { 1.0 },
    })
}

/// Fixed centrelines (index into the grid) that flow into this band's cells (`inflow`:
/// fixed cell, cell it drains into) onto dry ground that is no basin: cut at the seam.
pub(super) fn cut_at_seam(inflow: &[(usize, usize)], flags: &[u8], body: &[u32]) -> Vec<usize> {
    inflow
        .iter()
        .filter(|&&(i, r)| flags[i] & CENTRE != 0 && flags[r] & (RIVER | SEA) == 0 && body[r] == 0)
        .map(|&(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Owned = (Vec<f32>, Vec<f32>, Vec<u32>, Vec<u8>);

    /// Two 4-wide bands sharing two core rows, then two halo rows: a lake (id 2) on the left,
    /// a river (id 3) flowing south across the seam in column 2.
    fn rows() -> Owned {
        let (w, h) = (4, 4);
        let mut g = vec![10.0; w * h];
        let mut s = vec![f32::NAN; w * h];
        let mut b = vec![0; w * h];
        let mut f = vec![0; w * h];
        for y in 0..h {
            (g[y * w], s[y * w], b[y * w]) = (4.0, 5.0, 2);
            let i = y * w + 2;
            (g[i], s[i], b[i], f[i]) = (8.0, 9.0, 3, FLOW_CENTRE | 2);
        }
        (g, s, b, f)
    }

    /// Checks failing after `mutate` changes the southern band.
    fn failed(mutate: fn(&mut Owned)) -> Vec<String> {
        let (a, mut b) = (rows(), rows());
        mutate(&mut b);
        let kinds = HashMap::from([(2, BodyKind::Lake), (3, BodyKind::River)]);
        let r = compare(
            4,
            0,
            2,
            (&a.0, &a.1, &a.2, &a.3),
            (&b.0, &b.1, &b.2, &b.3),
            &kinds,
        );
        r["checks"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, v)| v.as_u64() != Some(0))
            .map(|(k, _)| k.clone())
            .collect()
    }

    #[test]
    fn every_seam_check_can_fail() {
        assert!(failed(|_| {}).is_empty(), "{:?}", failed(|_| {}));
        type Mutation = fn(&mut Owned);
        let cases: [(&str, Mutation); 6] = [
            ("shared_core_ground_differs", |r| r.0[5] = 11.0),
            ("shared_core_surface_differs", |r| r.1[0] = 5.5),
            ("shared_core_body_differs", |r| r.2[2] = 4),
            // The river runs onto dry ground below the seam.
            ("centrelines_cut_at_seam", |r| {
                (r.1[10], r.2[10]) = (f32::NAN, 0)
            }),
            // Dry ground below the lake beside it across the seam.
            ("dry_cells_below_lake_or_sea_across_seam", |r| {
                (r.0[9], r.1[9], r.2[9]) = (4.5, f32::NAN, 0);
                (r.0[8], r.1[8], r.2[8]) = (4.5, f32::NAN, 0);
            }),
            ("lakes_with_two_levels_at_seam", |r| r.1[8] = 5.5),
        ];
        for (name, mutate) in cases {
            let got = failed(mutate);
            assert!(got.contains(&name.to_string()), "{name}: {got:?}");
        }
    }

    #[test]
    fn a_river_cut_at_the_seam_fails() {
        // A fixed centreline (cell 0) drains into cell 1.
        let mut flags = vec![CENTRE | RIVER, RIVER];
        let mut body = vec![3, 3];
        assert!(cut_at_seam(&[(0, 1)], &flags, &body).is_empty());
        (flags[1], body[1]) = (0, 0);
        assert_eq!(cut_at_seam(&[(0, 1)], &flags, &body), vec![0]);
    }
}
