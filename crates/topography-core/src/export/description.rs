//! The export's `description.json` and `description.md`: the world's description (kept on
//! full-map exports), the body table in export ids, and river lines and lake outlines traced
//! on the 4 m hydrology.

use super::water::{Bodies, Kind, WaterStats};
use crate::acquisition::{Result, digest, ensure, read_small, write_new};
use crate::world::hydrology::grid;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// Export NW corner in the layout plane, sample spacing and samples per side.
pub(super) struct Frame {
    pub origin: [f64; 2],
    pub spacing_m: f64,
    pub size: usize,
}

impl Frame {
    fn extent_m(&self) -> f64 {
        self.size as f64 * self.spacing_m
    }
}

/// River width W = 2.5 · Q^0.5 m from discharge Q in km², the hydrology's rule
/// (`world::hydrology::rivers::WIDTH_A`).
const RIVER_WIDTH_A: f64 = 2.5;
/// River polylines keep every fourth 4 m node plus the ends.
const NODE_STRIDE: usize = 4;
/// Rows per read of the 4 m rasters.
const CHUNK_ROWS: usize = 256;
/// `flow.u8` centreline bit; the low 4 bits are the D8 receiver.
const FLOW_CENTRE: u8 = 16;
/// (dy, dx) of D8 receivers 1..=8: N, S, W, E, NW, NE, SW, SE.
const D8: [(isize, isize); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];
const COORDINATES: &str = "metres from the north-west corner, x east, y south";

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Sea => "sea",
        Kind::Lake => "lake",
        Kind::River => "river",
    }
}

fn round(v: f64, places: i32) -> f64 {
    let s = 10f64.powi(places);
    (v * s).round() / s
}

fn manifest_at(dir: &Path) -> Option<Value> {
    serde_json::from_slice(&read_small(&dir.join("manifest.json")).ok()?).ok()
}

/// A non-empty path field, resolved against `dir`.
fn path_in(dir: &Path, v: &Value) -> Option<PathBuf> {
    v.as_str().filter(|s| !s.is_empty()).map(|p| dir.join(p))
}

/// The first readable `description.json` of the world, its source world, or the layout.
fn find_base(world: &Path, manifest: &Value) -> Option<(Map<String, Value>, PathBuf)> {
    let source = path_in(world, &manifest["source_world"]);
    let source_manifest = source.as_deref().and_then(manifest_at);
    let layout = path_in(world, &manifest["layout"]).or_else(|| {
        let (dir, m) = (source.as_deref()?, source_manifest.as_ref()?);
        path_in(dir, &m["layout"])
    });
    [Some(world.to_path_buf()), source, layout]
        .into_iter()
        .flatten()
        .find_map(|dir| {
            let path = dir.join("description.json");
            let bytes = grid::read_bodies(&path).ok()?;
            let map = serde_json::from_slice::<Map<String, Value>>(&bytes).ok()?;
            Some((map, path))
        })
}

/// The 4 m hydrology directory: the world's `hydrology.4m`, else its source world's.
fn hydrology_dir(world: &Path, manifest: &Value) -> Option<PathBuf> {
    if let Some(dir) = path_in(world, &manifest["hydrology"]["4m"]) {
        return Some(dir);
    }
    let source = path_in(world, &manifest["source_world"])?;
    let m = manifest_at(&source)?;
    path_in(&source, &m["hydrology"]["4m"])
}

/// A 4 m centreline cell.
#[derive(Clone, Copy)]
struct Cell {
    flow: u8,
    body: u32,
    surface: f32,
    discharge: f32,
}

/// The same-body centreline cell that cell `i` of a `w`-wide grid drains into.
fn downstream(centre: &HashMap<usize, Cell>, w: usize, i: usize) -> Option<usize> {
    let cell = centre.get(&i)?;
    let d = usize::from(cell.flow & 15);
    let (dy, dx) = *D8.get(d.checked_sub(1)?)?;
    let r = (i / w).checked_add_signed(dy)?;
    let c = (i % w).checked_add_signed(dx)?;
    let j = r * w + c;
    (c < w && centre.get(&j)?.body == cell.body).then_some(j)
}

fn step_cells(flow: u8) -> f64 {
    if flow & 15 > 4 {
        std::f64::consts::SQRT_2
    } else {
        1.0
    }
}

/// Centreline paths (cell indices) and their world body, from each head (a cell no
/// same-body cell drains into), heads with the largest discharge-weighted length to the
/// mouth first. A path ends where its body or the centreline ends, or at a cell an earlier
/// path holds (the confluence, included).
fn paths(centre: &HashMap<usize, Cell>, w: usize) -> Vec<(u32, Vec<usize>)> {
    let next = |i| downstream(centre, w, i);
    // Discharge-weighted length from each cell to the end of its line.
    let mut weight: HashMap<usize, f64> = HashMap::with_capacity(centre.len());
    for &start in centre.keys() {
        let (mut stack, mut on) = (Vec::new(), HashSet::new());
        let (mut at, mut sum) = (Some(start), 0.0);
        while let Some(i) = at {
            if let Some(&v) = weight.get(&i) {
                sum = v;
                break;
            }
            // A receiver loop would never end; it counts as the end of the line.
            if !on.insert(i) {
                break;
            }
            stack.push(i);
            at = next(i);
        }
        for i in stack.into_iter().rev() {
            let c = centre[&i];
            sum += f64::from(c.discharge.max(0.0)) * step_cells(c.flow);
            weight.insert(i, sum);
        }
    }
    let fed: HashSet<usize> = centre.keys().filter_map(|&i| next(i)).collect();
    let mut heads: Vec<usize> = centre
        .keys()
        .copied()
        .filter(|i| !fed.contains(i))
        .collect();
    heads.sort_by(|a, b| weight[b].total_cmp(&weight[a]).then(a.cmp(b)));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for head in heads {
        let mut path = Vec::new();
        let mut at = Some(head);
        while let Some(i) = at {
            path.push(i);
            if !seen.insert(i) {
                break;
            }
            at = next(i);
        }
        out.push((centre[&head].body, path));
    }
    out
}

/// Cell-edge boundary rings of a row-major mask `w` wide, in cell-corner units (x east,
/// y down), each simplified by Douglas-Peucker at one cell and closed (first point repeated).
/// Cells touching only at a corner get separate rings; holes are rings of their own.
fn trace(mask: &[bool], w: usize) -> Vec<Vec<[f64; 2]>> {
    // Directions 0 E, 1 S, 2 W, 3 N; each edge keeps its cell on the right.
    const STEP: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
    let h = mask.len() / w.max(1);
    let at = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && mask[y as usize * w + x as usize]
    };
    let mut all = BTreeSet::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !at(x, y) {
                continue;
            }
            for (open, edge) in [
                (!at(x, y - 1), (y, x, 0u8)),
                (!at(x + 1, y), (y, x + 1, 1)),
                (!at(x, y + 1), (y + 1, x + 1, 2)),
                (!at(x - 1, y), (y + 1, x, 3)),
            ] {
                if open {
                    all.insert(edge);
                }
            }
        }
    }
    // The next edge turns right where it can, so the successor is a bijection on edges.
    let successor = |(y, x, d): (i64, i64, u8)| {
        let (dx, dy) = STEP[usize::from(d)];
        let (x, y) = (x + dx, y + dy);
        [(d + 1) % 4, d, (d + 3) % 4]
            .into_iter()
            .map(|d| (y, x, d))
            .find(|e| all.contains(e))
    };
    let mut remaining = all.clone();
    let mut rings = Vec::new();
    while let Some(first) = remaining.pop_first() {
        let mut edges = vec![first];
        let mut e = first;
        while let Some(n) = successor(e).filter(|&n| n != first) {
            remaining.remove(&n);
            edges.push(n);
            e = n;
        }
        let corners: Vec<[f64; 2]> = (0..edges.len())
            .filter(|&k| edges[k].2 != edges[(k + edges.len() - 1) % edges.len()].2)
            .map(|k| [edges[k].1 as f64, edges[k].0 as f64])
            .collect();
        rings.push(simplify_ring(&corners, 1.0));
    }
    rings
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// Douglas-Peucker on an open line; keeps both ends.
fn simplify_line(line: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    let n = line.len();
    if n < 3 {
        return line.to_vec();
    }
    let mut keep = vec![false; n];
    (keep[0], keep[n - 1]) = (true, true);
    let mut spans = vec![(0, n - 1)];
    while let Some((a, b)) = spans.pop() {
        let far = (a + 1..b)
            .map(|k| (k, segment_distance(line[k], line[a], line[b])))
            .max_by(|p, q| p.1.total_cmp(&q.1));
        if let Some((k, _)) = far.filter(|f| f.1 > eps) {
            keep[k] = true;
            spans.extend([(a, k), (k, b)]);
        }
    }
    line.iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// Douglas-Peucker on a ring of distinct corners, split at the corner farthest from the
/// first; returns it closed. Rings that would keep fewer than four corners stay as they are.
fn simplify_ring(ring: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    let closed = |r: &[[f64; 2]]| {
        let mut r = r.to_vec();
        r.extend(r.first().copied());
        r
    };
    let d2 = |p: [f64; 2]| (p[0] - ring[0][0]).powi(2) + (p[1] - ring[0][1]).powi(2);
    let Some(far) = (1..ring.len()).max_by(|&a, &b| d2(ring[a]).total_cmp(&d2(ring[b]))) else {
        return closed(ring);
    };
    let mut out = simplify_line(&ring[..=far], eps);
    out.pop();
    let back: Vec<[f64; 2]> = ring[far..].iter().chain(&ring[..1]).copied().collect();
    out.extend(simplify_line(&back, eps));
    if out.len() < 5 { closed(ring) } else { out }
}

/// Rivers and lakes of the export from the 4 m hydrology in `dir`.
struct Water {
    rivers: Vec<Value>,
    lakes: Vec<Value>,
}

fn water(dir: &Path, frame: &Frame, bodies: &Bodies, stats: &WaterStats) -> Result<Water> {
    let m: Map<String, Value> = serde_json::from_slice(&read_small(&dir.join("grid.json"))?)?;
    let g = grid::shape(&m)?;
    let (w, s, nw) = (g.w, g.spacing_m, g.nw_m);
    let size = [g.w, g.h];
    let extent = frame.extent_m();
    // Map metres of a cell corner (c, r) of the 4 m grid.
    let map_m = |c: f64, r: f64| {
        [
            nw[0] + c * s - frame.origin[0],
            frame.origin[1] - nw[1] + r * s,
        ]
    };
    // 4 m cells whose centre lies in the export: [first, end) per axis.
    let span = |offset: f64, n: usize| {
        let first = (offset / s - 0.5).ceil().max(0.0) as usize;
        let end = ((offset + extent) / s - 0.5).ceil().clamp(0.0, n as f64) as usize;
        [first.min(end), end]
    };
    let [c0, c1] = span(frame.origin[0] - nw[0], g.w);
    let [r0, r1] = span(nw[1] - frame.origin[1], g.h);
    let present = |b: u32| {
        bodies
            .export_id(b)
            .filter(|&id| stats.cells[usize::from(id)] > 0)
    };
    let mut centre = HashMap::new();
    let mut lake_bounds: BTreeMap<u16, [usize; 4]> = BTreeMap::new();
    let f32le = f32::from_le_bytes;
    let rows = if c0 < c1 { r0..r1 } else { 0..0 };
    for y0 in rows.step_by(CHUNK_ROWS) {
        let rows = [y0, (y0 + CHUNK_ROWS).min(r1)];
        let window = |name: &str| dir.join(name);
        let flow = grid::read_window(&window("flow.u8"), size, [c0, c1], rows, |b: [u8; 1]| b[0])?;
        let body = grid::read_window(
            &window("water-body.u32"),
            size,
            [c0, c1],
            rows,
            u32::from_le_bytes,
        )?;
        let surface = grid::read_window(&window("water-surface.f32"), size, [c0, c1], rows, f32le)?;
        let discharge = grid::read_window(&window("discharge.f32"), size, [c0, c1], rows, f32le)?;
        for (k, (&f, &b)) in flow.iter().zip(&body).enumerate() {
            let Some(id) = present(b) else { continue };
            let (c, r) = (c0 + k % (c1 - c0), y0 + k / (c1 - c0));
            match bodies.kind[usize::from(id)] {
                Kind::River if f & FLOW_CENTRE != 0 => {
                    let cell = Cell {
                        flow: f,
                        body: b,
                        surface: surface[k],
                        discharge: discharge[k],
                    };
                    centre.insert(r * w + c, cell);
                }
                Kind::Lake => {
                    let e = lake_bounds
                        .entry(id)
                        .or_insert([usize::MAX, usize::MAX, 0, 0]);
                    *e = [e[0].min(c), e[1].min(r), e[2].max(c + 1), e[3].max(r + 1)];
                }
                _ => {}
            }
        }
    }

    let listed: Value = serde_json::from_slice(&grid::read_bodies(&dir.join("bodies.json"))?)?;
    let info: HashMap<u64, &Value> = listed["bodies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| Some((b["id"].as_u64()?, b)))
        .collect();
    let point = |p: [f64; 2]| [round(p[0], 2), round(p[1], 2)];
    let centre_m = |i: usize| map_m((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
    let mut by_body: BTreeMap<u16, Vec<Vec<usize>>> = BTreeMap::new();
    for (b, path) in paths(&centre, w) {
        if path.len() >= 2 {
            let id = bodies.export_id(b).unwrap_or_default();
            by_body.entry(id).or_default().push(path);
        }
    }
    let mut rivers = Vec::new();
    for (id, list) in by_body {
        let world_id = bodies.world_id[usize::from(id)];
        let length = |p: &Vec<usize>| {
            p.windows(2)
                .map(|n| step_cells(centre[&n[0]].flow))
                .sum::<f64>()
        };
        let longest = list
            .iter()
            .max_by(|a, b| length(a).total_cmp(&length(b)))
            .map(|p| point(centre_m(p[0])));
        let lines: Vec<Value> = list
            .iter()
            .map(|path| {
                let n = path.len();
                let nodes: Vec<Value> = (0..n)
                    .filter(|k| k % NODE_STRIDE == 0 || k + 1 == n)
                    .map(|k| {
                        let c = centre[&path[k]];
                        let [x, y] = point(centre_m(path[k]));
                        let q = f64::from(c.discharge.max(0.0));
                        json!([
                            x,
                            y,
                            round(RIVER_WIDTH_A * q.sqrt(), 2),
                            round(f64::from(c.surface), 3),
                            round(q, 4),
                        ])
                    })
                    .collect();
                Value::Array(nodes)
            })
            .collect();
        let entry = info.get(&u64::from(world_id));
        let mouth = entry
            .and_then(|e| serde_json::from_value::<[f64; 2]>(e["mouth_cell"].clone()).ok())
            .map(|[c, r]| map_m(c + 0.5, r + 0.5))
            .filter(|p| p.iter().all(|v| (0.0..=extent).contains(v)))
            .map(point);
        rivers.push(json!({
            "body": id,
            "length_km": entry.and_then(|e| e["length_km"].as_f64()).map(|v| round(v, 3)),
            "source_m": longest,
            "mouth_m": mouth,
            "paths_m": lines,
        }));
    }

    let mut lakes = Vec::new();
    for (id, [x0, y0, x1, y1]) in lake_bounds {
        let world_id = bodies.world_id[usize::from(id)];
        let cells =
            grid::read_window(&dir.join("water-body.u32"), size, [x0, x1], [y0, y1], |b| {
                u32::from_le_bytes(b) == world_id
            })?;
        let rings: Vec<Value> = trace(&cells, x1 - x0)
            .into_iter()
            .map(|ring| {
                let pts: Vec<[f64; 2]> = ring
                    .into_iter()
                    .map(|[i, j]| {
                        let [x, y] = map_m(x0 as f64 + i, y0 as f64 + j);
                        point([x.clamp(0.0, extent), y.clamp(0.0, extent)])
                    })
                    .collect();
                json!(pts)
            })
            .collect();
        let i = usize::from(id);
        lakes.push(json!({
            "body": id,
            "level_m": bodies.level_m[i],
            "area_km2": round(area_km2(stats, i, frame), 6),
            "outline_m": rings,
        }));
    }
    Ok(Water { rivers, lakes })
}

fn area_km2(stats: &WaterStats, i: usize, frame: &Frame) -> f64 {
    stats.cells[i] as f64 * frame.spacing_m * frame.spacing_m / 1e6
}

/// The body table in export ids: every id of the `water-body` tiles.
fn body_table(bodies: &Bodies, stats: &WaterStats, frame: &Frame) -> Vec<Value> {
    (1..bodies.len())
        .map(|i| {
            let s = frame.spacing_m;
            let bounds = (stats.cells[i] > 0).then(|| stats.bounds[i].map(|v| v as f64 * s));
            let mut e = json!({
                "id": i,
                "world_id": bodies.world_id[i],
                "kind": kind_name(bodies.kind[i]),
                "area_km2": round(area_km2(stats, i, frame), 6),
                "bounds_m": bounds,
                "parent": bodies.parent[i],
            });
            if bodies.level_m[i].is_finite() {
                e["level_m"] = json!(bodies.level_m[i]);
            }
            e
        })
        .collect()
}

fn format_point(p: &Value) -> String {
    match serde_json::from_value::<[f64; 2]>(p.clone()) {
        Ok([x, y]) => format!("{x:.0}, {y:.0}"),
        Err(_) => "—".into(),
    }
}

/// The water section of `description.md`.
fn water_markdown(d: &Map<String, Value>, bodies: &Bodies, stats: &WaterStats) -> String {
    let count = |k: Kind| {
        (1..bodies.len())
            .filter(|&i| bodies.kind[i] == k && stats.cells[i] > 0)
            .count()
    };
    let mut md = format!(
        "## Water\n\nBodies in the export by kind: sea {}, lake {}, river {}. Ids are export ids, as in \
         the `water-body` tiles; `description.json` maps them to world bodies.\n",
        count(Kind::Sea),
        count(Kind::Lake),
        count(Kind::River),
    );
    let label = |id: &Value| {
        let i = id.as_u64().unwrap_or(0) as usize;
        format!(
            "export id {i} (world body {})",
            bodies.world_id.get(i).copied().unwrap_or(0)
        )
    };
    let list = |key: &str| {
        d.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let rivers = list("rivers");
    if !rivers.is_empty() {
        md += "\n### Rivers\n\n| Body | Length km | Source m | Mouth m |\n| --- | ---: | --- | --- |\n";
        for r in &rivers {
            let length = r["length_km"]
                .as_f64()
                .map_or("—".into(), |v| format!("{v:.2}"));
            md += &format!(
                "| {} | {length} | {} | {} |\n",
                label(&r["body"]),
                format_point(&r["source_m"]),
                format_point(&r["mouth_m"]),
            );
        }
    }
    let lakes = list("lakes");
    if !lakes.is_empty() {
        md += "\n### Lakes\n\n| Body | Level m | Area km² |\n| --- | ---: | ---: |\n";
        for l in &lakes {
            md += &format!(
                "| {} | {:.1} | {:.3} |\n",
                label(&l["body"]),
                l["level_m"].as_f64().unwrap_or(f64::NAN),
                l["area_km2"].as_f64().unwrap_or(0.0),
            );
        }
    }
    md
}

/// Writes description.json and description.md into the stage; returns description.json's SHA-256.
pub(super) fn write(
    stage: &Path,
    world: &Path,
    manifest: &Value,
    frame: &Frame,
    bodies: &Bodies,
    stats: &WaterStats,
) -> Result<String> {
    ensure(
        bodies.world_id.len() == bodies.len() && stats.cells.len() == bodies.len(),
        "water statistics do not match the body table",
    )?;
    let input = PathBuf::from(manifest["world_input"].as_str().unwrap_or_default());
    let name = read_small(&input.join("world.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["name"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let extent = frame.extent_m();
    let mut notes = Vec::new();
    let base = find_base(world, manifest);
    let kept = base.as_ref().filter(|(b, _)| {
        b.get("extent_m")
            .and_then(Value::as_f64)
            .is_some_and(|e| (e - extent).abs() < 1e-6)
    });
    let (mut d, base_md) = match kept {
        Some((b, path)) => {
            let md = fs::read_to_string(path.with_extension("md")).ok();
            let mut d = b.clone();
            if let Some(note) = d.remove("note").and_then(|n| n.as_str().map(str::to_owned)) {
                notes.push(note);
            }
            (d, md)
        }
        None => {
            notes.push(if base.is_some() {
                "layout geometry left out: crop export".to_owned()
            } else {
                "layout geometry left out: no world description found".to_owned()
            });
            let d = json!({
                "kind": "world_description",
                "schema_version": 1,
                "world": name,
                "regions": [],
                "passes": [],
                "features": [],
            });
            (d.as_object().cloned().unwrap_or_default(), None)
        }
    };
    let measured = d.get("level").cloned().unwrap_or(Value::Null);
    let hydrology = hydrology_dir(world, manifest).filter(|dir| dir.join("grid.json").is_file());
    let water = match &hydrology {
        Some(dir) => water(dir, frame, bodies, stats)?,
        None => {
            notes.push("no 4 m hydrology: rivers and lakes have no lines".to_owned());
            Water {
                rivers: Vec::new(),
                lakes: Vec::new(),
            }
        }
    };
    for (key, value) in [
        ("kind", json!("world_description")),
        ("schema_version", json!(1)),
        ("level", json!("export")),
        ("measured_level", measured),
        ("cell_m", json!(frame.spacing_m)),
        ("extent_m", json!(extent)),
        ("coordinates", json!(COORDINATES)),
        ("water_bodies", json!(body_table(bodies, stats, frame))),
        ("rivers", json!(water.rivers)),
        ("lakes", json!(water.lakes)),
    ] {
        d.insert(key.into(), value);
    }
    if !notes.is_empty() {
        d.insert("note".into(), json!(notes.join("; ")));
    }
    let bytes = serde_json::to_vec(&d)?;
    write_new(&stage.join("description.json"), &bytes)?;
    let mut md = base_md.unwrap_or_else(|| {
        format!(
            "# {name}: world description\n\nLevel `export`, {} m cells, {:.1} km square. \
             Positions are {COORDINATES}.\n",
            frame.spacing_m,
            extent / 1000.0
        )
    });
    if !md.ends_with('\n') {
        md.push('\n');
    }
    if !notes.is_empty() {
        md += &format!("\nNote: {}.\n", notes.join("; "));
    }
    md += "\n";
    md += &water_markdown(&d, bodies, stats);
    write_new(&stage.join("description.md"), md.as_bytes())?;
    Ok(digest(&bytes))
}

#[cfg(test)]
mod tests {
    use super::super::{Layout, heightmap, testworld, tile_path, view::read_png};
    use super::*;

    fn export(world: &Path, out: &Path) -> (Value, Value) {
        heightmap(world, out, 16, Layout::SharedEdge, false).unwrap();
        let read = |name: &str| -> Value {
            serde_json::from_slice(&fs::read(out.join(name)).unwrap()).unwrap()
        };
        (read("manifest.json"), read("description.json"))
    }

    #[test]
    fn bodies_table_uses_export_ids_and_types() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, y| match (x, y) {
            (0..40, _) => (-5.0, f32::NAN, 1),
            (100..140, 100..140) => (10.0, f32::NAN, 7),
            (200, _) => (15.0, 16.0, 40000),
            _ => (20.0, f32::NAN, 0),
        });
        let out = dir.path().join("x");
        let (m, d) = export(&world, &out);
        let table = d["water_bodies"].as_array().unwrap();
        let col = |k: &str| -> Vec<Value> { table.iter().map(|b| b[k].clone()).collect() };
        assert_eq!(col("id"), [json!(1), json!(2), json!(3)]);
        assert_eq!(col("world_id"), [json!(1), json!(7), json!(40000)]);
        assert_eq!(col("kind"), [json!("sea"), json!("lake"), json!("river")]);
        assert_eq!(table[1]["bounds_m"], json!([100.0, 100.0, 140.0, 140.0]));
        assert_eq!(table[1]["level_m"], json!(11.0));
        assert!(table[2].get("level_m").is_none());
        assert_eq!(d["level"], "export");
        assert!(d["note"].as_str().unwrap().contains("no 4 m hydrology"));
        // L0 sample (120, 120) is texel (8, 8) of tile (7, 7), plus the apron of 4.
        let (w, _, px) = read_png(&out.join(tile_path("water-body", 0, 7, 7))).unwrap();
        let i = 12 * w + 12;
        assert_eq!(u16::from_be_bytes([px[2 * i], px[2 * i + 1]]), 2);
        let sha = digest(&fs::read(out.join("description.json")).unwrap());
        assert_eq!(m["description"]["sha256"], json!(sha));
        assert_eq!(m["description"]["schema"], "world-description-v1");
        let md = fs::read_to_string(out.join("description.md")).unwrap();
        assert!(md.contains("sea 1, lake 1, river 1"));
    }

    #[test]
    fn full_map_base_description_is_kept_and_crops_drop_it() {
        for (extent, kept) in [(256.0, true), (1024.0, false)] {
            let dir = tempfile::tempdir().unwrap();
            let world = testworld::write(dir.path(), 256, 1.0, |x, _| (x as f32, f32::NAN, 0));
            let base = json!({"kind": "world_description", "schema_version": 1, "world": "w",
                "level": "l3", "cell_m": 4.0, "extent_m": extent,
                "regions": [{"id": "a"}], "passes": [{"id": "p"}], "features": [{"id": "f"}]});
            fs::write(world.join("description.json"), base.to_string()).unwrap();
            fs::write(world.join("description.md"), "# w: world description\n").unwrap();
            let out = dir.path().join("x");
            let (_, d) = export(&world, &out);
            assert_eq!(d["level"], "export");
            assert_eq!(d["extent_m"], 256.0);
            let md = fs::read_to_string(out.join("description.md")).unwrap();
            assert!(md.contains("## Water"));
            if kept {
                assert_eq!(d["regions"], json!([{"id": "a"}]));
                assert_eq!(d["measured_level"], "l3");
                assert_eq!(d["cell_m"], 1.0);
                assert!(md.starts_with("# w: world description\n"));
            } else {
                for key in ["regions", "passes", "features"] {
                    assert_eq!(d[key], json!([]), "{key}");
                }
                assert_eq!(d["world"], "testworld");
                assert!(d["note"].as_str().unwrap().contains("crop export"));
                assert!(d["measured_level"].is_null());
            }
        }
    }

    fn area(ring: &[[f64; 2]]) -> f64 {
        let s: f64 = ring
            .windows(2)
            .map(|p| p[0][0] * p[1][1] - p[1][0] * p[0][1])
            .sum();
        (s / 2.0).abs()
    }

    #[test]
    fn lake_outline_encloses_its_cells() {
        // An L: a 6 by 2 bar with a 2 by 4 leg.
        let w = 10;
        let mut mask = vec![false; w * 9];
        let mut set = |x: usize, y: usize| mask[y * w + x] = true;
        for y in 1..3 {
            (1..7).for_each(|x| set(x, y));
        }
        for y in 3..7 {
            (1..3).for_each(|x| set(x, y));
        }
        let cells = mask.iter().filter(|&&m| m).count() as f64;
        let rings = trace(&mask, w);
        assert_eq!(rings.len(), 1);
        let ring = &rings[0];
        assert_eq!(ring.first(), ring.last());
        assert!(
            (area(ring) - cells).abs() <= 1.0,
            "{} vs {cells}",
            area(ring)
        );
        // Corner-touching cells are separate rings; a hole is a ring of its own.
        let mut mask = vec![false; 25];
        for (x, y) in [(0, 0), (1, 1)] {
            mask[y * 5 + x] = true;
        }
        assert_eq!(trace(&mask, 5).len(), 2);
        let ring: Vec<bool> = (0..25).map(|i| i != 12).collect();
        let rings = trace(&ring, 5);
        assert_eq!(rings.len(), 2);
        assert_eq!(rings.iter().map(|r| area(r)).sum::<f64>(), 26.0);
    }

    #[test]
    fn river_paths_run_longest_first_and_stop_at_the_confluence() {
        // A 10-cell main stem down column 2 (flow S) and a 2-cell tributary joining at row 5
        // from the east (flow W), all world body 9; a body 8 cell below the mouth.
        let w = 5;
        let mut centre = HashMap::new();
        let mut put = |c: usize, r: usize, dir: u8, body: u32, q: f32| {
            let cell = Cell {
                flow: FLOW_CENTRE | dir,
                body,
                surface: 1.0,
                discharge: q,
            };
            centre.insert(r * w + c, cell);
        };
        for r in 0..10 {
            put(2, r, 2, 9, 1.0 + r as f32);
        }
        put(4, 5, 3, 9, 0.5);
        put(3, 5, 3, 9, 0.6);
        put(2, 10, 0, 8, 20.0);
        let paths = paths(&centre, w);
        let main: Vec<usize> = (0..10).map(|r| r * w + 2).collect();
        assert_eq!(paths[0], (9, main));
        assert!(paths.contains(&(9, vec![5 * w + 4, 5 * w + 3, 5 * w + 2])));
        assert!(paths.contains(&(8, vec![10 * w + 2])));
        assert_eq!(paths.len(), 3);
    }
}
