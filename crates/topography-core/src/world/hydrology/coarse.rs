//! The coarser level a finer grid lies in (`--coarse`, the previous level's `--out`): its
//! water, flow, discharge and body list over the window the finer grid covers,
//! and its moisture grid.

use super::flood::{Dims, FLOW_CENTRE, FLOW_DRY, FLOW_SINK};
use super::grid::{self, Grid};
use crate::acquisition::{AcquisitionError, Result, digest, ensure, io_context, read_small};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

pub(super) struct Lake {
    pub level: f32,
    /// Coarse cell (column, row) its flow leaves by.
    pub exit: Option<[usize; 2]>,
}

pub(super) struct River {
    /// Body its water reaches (1 sea, a lake or dry basin id, 0 the grid edge).
    pub parent: u32,
    pub drainage_km2: f32,
}

/// Moisture on its own grid (128 m), reused by every finer level.
pub(super) struct Moisture {
    pub w: usize,
    pub h: usize,
    pub spacing_m: f64,
    pub nw_m: [f64; 2],
    pub values: Vec<f32>,
    /// Discharge weight per moisture unit: scale_mm_h / mean_land_mm_h of the level that made it.
    pub weight: f32,
    pub windless: bool,
    pub wind_from_deg: Option<f64>,
    /// The coarse grid.json `moisture` entry, passed on.
    pub entry: Map<String, Value>,
}

impl Moisture {
    /// Bilinear between cell centres at a point of the layout plane.
    pub fn at(&self, x: f64, y: f64) -> f32 {
        let fx = ((x - self.nw_m[0]) / self.spacing_m - 0.5).clamp(0.0, (self.w - 1) as f64);
        let fy = ((self.nw_m[1] - y) / self.spacing_m - 0.5).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (ax, ay) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let p = |x: usize, y: usize| self.values[y * self.w + x];
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bottom * ay
    }
}

pub(super) struct Coarse {
    /// Fine cells per coarse cell side.
    pub k: usize,
    /// Whole coarse grid in cells.
    pub size: [usize; 2],
    /// Fine grid's north-west corner in fine cells from the coarse north-west corner.
    pub off: [usize; 2],
    /// Window read, in coarse cells: first column and row, width and height.
    pub x0: usize,
    pub y0: usize,
    pub ww: usize,
    pub wh: usize,
    pub surface: Vec<f32>,
    pub body: Vec<u32>,
    pub flow: Vec<u8>,
    pub discharge: Vec<f32>,
    pub lakes: HashMap<u32, Lake>,
    pub rivers: HashMap<u32, River>,
    pub dry: HashSet<u32>,
    pub max_id: u32,
    pub moisture: Moisture,
    pub spacing_m: f64,
    pub digests: Map<String, Value>,
}

impl Coarse {
    /// Window index of the coarse cell holding fine cell `i` of a `fine_w`-wide grid.
    pub fn cell(&self, fine_w: usize, i: usize) -> usize {
        let cx = (self.off[0] + i % fine_w) / self.k - self.x0;
        let cy = (self.off[1] + i / fine_w) / self.k - self.y0;
        cy * self.ww + cx
    }
    /// Whole-grid column and row of window cell `q`.
    pub fn abs(&self, q: usize) -> [usize; 2] {
        [self.x0 + q % self.ww, self.y0 + q / self.ww]
    }
    /// Window cell at a whole-grid column and row, if read.
    pub fn at(&self, [x, y]: [isize; 2]) -> Option<usize> {
        let (x, y) = (x - self.x0 as isize, y - self.y0 as isize);
        (x >= 0 && y >= 0 && (x as usize) < self.ww && (y as usize) < self.wh)
            .then(|| y as usize * self.ww + x as usize)
    }
    pub fn dims(&self) -> Dims {
        Dims {
            w: self.ww,
            h: self.wh,
        }
    }
    pub fn dir(&self, q: usize) -> u8 {
        self.flow[q] & 15
    }
    pub fn centre(&self, q: usize) -> bool {
        self.flow[q] & FLOW_CENTRE != 0
    }
    pub fn dry_basin(&self, q: usize) -> Option<bool> {
        (self.flow[q] & FLOW_DRY != 0).then_some(self.flow[q] & FLOW_SINK != 0)
    }
    pub fn lake(&self, q: usize) -> Option<u32> {
        Some(self.body[q]).filter(|b| self.lakes.contains_key(b))
    }
    pub fn river(&self, q: usize) -> Option<u32> {
        Some(self.body[q]).filter(|b| self.rivers.contains_key(b))
    }
    /// Fine cells (index range per row) of window cell `q` inside a `w` x `h` fine grid.
    pub fn fine_cells(&self, q: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
        let [cx, cy] = self.abs(q);
        let span = |c: usize, off: usize, n: usize| {
            let a = (c * self.k).saturating_sub(off).min(n);
            let b = ((c + 1) * self.k).saturating_sub(off).min(n);
            a..b
        };
        let xs = span(cx, self.off[0], w);
        span(cy, self.off[1], h).flat_map(move |y| xs.clone().map(move |x| y * w + x))
    }
    /// Fine cell `i` of a `w` x `h` grid lies on a side that is the world's edge.
    pub fn world_edge(&self, w: usize, h: usize, i: usize) -> bool {
        let (x, y) = (i % w, i / w);
        let [cw, ch] = [self.size[0] * self.k, self.size[1] * self.k];
        (y == 0 && self.off[1] == 0)
            || (y + 1 == h && self.off[1] + h == ch)
            || (x == 0 && self.off[0] == 0)
            || (x + 1 == w && self.off[0] + w == cw)
    }
    /// Coarse discharge entering fine edge cell `i` from outside the grid: a coarse cell
    /// across the edge that drains into this cell's coarse cell, shared over the fine cells
    /// along that coarse side.
    pub fn edge_inflow(&self, w: usize, h: usize, i: usize) -> f32 {
        let (x, y) = (i % w, i / w);
        let [ax, ay] = [(self.off[0] + x) as isize, (self.off[1] + y) as isize];
        let k = self.k as isize;
        let here = [ax / k, ay / k];
        let mut q = 0.0;
        let sides = [
            (y == 0, [ax, ay - 1]),
            (y + 1 == h, [ax, ay + 1]),
            (x == 0, [ax - 1, ay]),
            (x + 1 == w, [ax + 1, ay]),
        ];
        for (on, [ox, oy]) in sides {
            if !on || ox < 0 || oy < 0 {
                continue;
            }
            let Some(p) = self.at([ox.div_euclid(k), oy.div_euclid(k)]) else {
                continue;
            };
            if self.abs(p) == [here[0] as usize, here[1] as usize] || self.dir(p) == 0 {
                continue;
            }
            let (dy, dx) = Dims::offset(self.dir(p));
            let [px, py] = self.abs(p);
            if [px as isize + dx, py as isize + dy] == here {
                q += self.discharge[p] / self.k as f32;
            }
        }
        q
    }
    /// Fine edge cell `i` of a `w` x `h` grid is an outlet: on a side that is the world's
    /// edge, or where its coarse cell drains out across that side.
    pub fn edge_outlet(&self, w: usize, h: usize, i: usize) -> bool {
        let (x, y) = (i % w, i / w);
        let (dy, dx) = match self.dir(self.cell(w, i)) {
            0 => (0, 0),
            d => Dims::offset(d),
        };
        let [cw, ch] = [self.size[0] * self.k, self.size[1] * self.k];
        (y == 0 && (self.off[1] == 0 || dy < 0))
            || (y + 1 == h && (self.off[1] + h == ch || dy > 0))
            || (x == 0 && (self.off[0] == 0 || dx < 0))
            || (x + 1 == w && (self.off[0] + w == cw || dx > 0))
    }
}

pub(super) fn read(dir: &Path, fine: &Grid) -> Result<Coarse> {
    let bytes = read_small(&dir.join("grid.json"))?;
    let manifest: Map<String, Value> = serde_json::from_slice(&bytes)?;
    let mut digests = Map::new();
    digests.insert("grid.json".into(), digest(&bytes).into());
    let g = grid::shape(&manifest)?;
    let ratio = g.spacing_m / fine.spacing_m;
    ensure(
        ratio >= 2.0 && (ratio - ratio.round()).abs() < 1e-9,
        format!(
            "--coarse spacing {} m must be a whole multiple (2 or more) of {} m",
            g.spacing_m, fine.spacing_m
        ),
    )?;
    let k = ratio.round() as usize;
    let cells = |metres: f64| -> Result<usize> {
        let c = metres / fine.spacing_m;
        ensure(
            c > -1e-6 && (c - c.round()).abs() < 1e-6,
            "the grid is not aligned to the --coarse grid (origin/offset_m)",
        )?;
        Ok(c.round() as usize)
    };
    let off = [
        cells(fine.nw_m[0] - g.nw_m[0])?,
        cells(g.nw_m[1] - fine.nw_m[1])?,
    ];
    ensure(
        off[0] + fine.w <= g.w * k && off[1] + fine.h <= g.h * k,
        "the --coarse grid does not cover the grid",
    )?;
    let x0 = (off[0] / k).saturating_sub(1);
    let y0 = (off[1] / k).saturating_sub(1);
    let x1 = ((off[0] + fine.w - 1) / k + 2).min(g.w);
    let y1 = ((off[1] + fine.h - 1) / k + 2).min(g.h);
    let size = [g.w, g.h];
    let f32le = |b: [u8; 4]| f32::from_le_bytes(b);
    let raster = |name: &str| dir.join(name);
    let flow_path = raster("flow.u8");
    ensure(
        flow_path.exists(),
        format!(
            "{} lacks flow.u8; rerun hydrology on the coarse level",
            dir.display()
        ),
    )?;
    let surface = grid::read_window(
        &raster("water-surface.f32"),
        size,
        [x0, x1],
        [y0, y1],
        f32le,
    )?;
    let body = grid::read_window(
        &raster("water-body.u32"),
        size,
        [x0, x1],
        [y0, y1],
        u32::from_le_bytes,
    )?;
    let flow = grid::read_window(&flow_path, size, [x0, x1], [y0, y1], |b: [u8; 1]| b[0])?;
    let discharge = grid::read_window(&raster("discharge.f32"), size, [x0, x1], [y0, y1], f32le)?;

    let bodies_bytes = read_small(&raster("bodies.json"))?;
    digests.insert("bodies.json".into(), digest(&bodies_bytes).into());
    let bodies: Value = serde_json::from_slice(&bodies_bytes)?;
    let (mut lakes, mut rivers, mut dry, mut max_id) =
        (HashMap::new(), HashMap::new(), HashSet::new(), 1u32);
    for b in bodies["bodies"].as_array().into_iter().flatten() {
        let id = b["id"].as_u64().unwrap_or(0) as u32;
        max_id = max_id.max(id);
        let cell = |key: &str| serde_json::from_value::<[usize; 2]>(b[key].clone()).ok();
        match b["kind"].as_str() {
            Some("lake") => {
                lakes.insert(
                    id,
                    Lake {
                        level: b["level_m"].as_f64().unwrap_or(f64::NAN) as f32,
                        exit: cell("exit_cell"),
                    },
                );
            }
            Some("river") => {
                rivers.insert(
                    id,
                    River {
                        parent: b["parent"].as_u64().unwrap_or(0) as u32,
                        drainage_km2: b["drainage_km2"].as_f64().unwrap_or(0.0) as f32,
                    },
                );
            }
            Some("dry") => {
                dry.insert(id);
            }
            _ => {}
        }
    }
    ensure(
        lakes.values().all(|l| l.level.is_finite()),
        "coarse bodies.json has a lake without level_m",
    )?;

    // Moisture: never recomputed on finer levels.
    let entry = manifest
        .get("moisture")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| AcquisitionError::Invalid("coarse grid.json lacks moisture".into()))?;
    let [mw, mh]: [usize; 2] = serde_json::from_value(entry["size"].clone())
        .map_err(|_| AcquisitionError::Invalid("coarse moisture size".into()))?;
    let m_spacing = entry["spacing_m"].as_f64().unwrap_or(f64::NAN);
    let nw_m = entry
        .get("nw_m")
        .and_then(|v| serde_json::from_value::<[f64; 2]>(v.clone()).ok())
        .unwrap_or(g.nw_m);
    let m_path = raster("moisture.f32");
    let m_bytes = io_context(format!("read {}", m_path.display()), fs::read(&m_path))?;
    ensure(
        m_bytes.len() == mw * mh * 4 && m_spacing > 0.0,
        "coarse moisture.f32 does not match its grid.json entry",
    )?;
    digests.insert("moisture.f32".into(), digest(&m_bytes).into());
    let values: Vec<f32> = m_bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let wind_from_deg = manifest.get("wind_from_deg").and_then(Value::as_f64);
    let scale = entry["scale_mm_h"].as_f64().unwrap_or(1.0);
    let mean = manifest
        .get("discharge")
        .and_then(|d| d["mean_land_mm_h"].as_f64())
        .unwrap_or(scale);
    let moisture = Moisture {
        w: mw,
        h: mh,
        spacing_m: m_spacing,
        nw_m,
        values,
        weight: (scale / mean.max(1e-9)) as f32,
        windless: wind_from_deg.is_none(),
        wind_from_deg,
        entry,
    };
    Ok(Coarse {
        k,
        size,
        off,
        x0,
        y0,
        ww: x1 - x0,
        wh: y1 - y0,
        surface,
        body,
        flow,
        discharge,
        lakes,
        rivers,
        dry,
        max_id,
        moisture,
        spacing_m: g.spacing_m,
        digests,
    })
}
