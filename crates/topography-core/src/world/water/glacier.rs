//! Glacier ice in place of lake ice at the snowline (PLAN-export-v10 item 16, owner decision
//! 2026-11-16). An ice lake at or above the layout snowline fills its bowl with plastic ice:
//! a surface that rises from a terminus a short way down the outlet (Nye: √(2τ/ρg · d)),
//! flattening toward a ceiling and meeting the walls where it reaches the ground. Lakes that
//! drain into each other form one glacier. Ice only covers cells that drain to the lake or
//! its outlet reach, so cells outside keep their ground and routing; transverse crevasses
//! are capped by the local slope, and a final pit fill guarantees the ice holds no water.

use super::geom::{OFFSETS, Rect, World, step_len};
use super::layers::Layer;
use crate::acquisition::{Result, ensure, io_context};
use serde_json::{Value, json};
use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::path::Path;

/// Bump on any change to the construction: it keys the cached glacier ground.
pub(crate) const VERSION: &str = "glacier v2";
/// Basal yield stress of plastic ice (kPa; 50 over 90 by hillshade: a steadier slope with
/// crevasses across the ice, where 90 flattens the interior), ice density, gravity.
const YIELD_KPA: f64 = 50.0;
const ICE_DENSITY: f64 = 900.0;
const GRAVITY: f64 = 9.81;
/// The terminus lies this far down the outlet path from the exit.
const TONGUE_M: f64 = 40.0;
/// Ice may rise this far above the Nye profile from its source (plastic ice creeping up
/// walls stops there), and is at least this thick.
const ENVELOPE_M: f64 = 10.0;
/// The surface flattens (tanh) toward this rise above the spill: this times √(lake area),
/// clamped, and at least 1.5 times the joined lakes' rise above it plus ENVELOPE_M (so
/// their ice still slopes).
const RISE_PER_M: f64 = 0.1;
const RISE_MIN_M: f64 = 10.0;
const RISE_MAX_M: f64 = 80.0;
const ICE_MIN_M: f64 = 1.0;
/// Crevasse spacing along flow, the largest crevasse relief, the thickness over which it
/// fades in, and the share of `slope · spacing` it may reach (keeps the surface descending
/// along flow; the profile's back slope is 1.25 / spacing).
const CREVASSE_M: f64 = 12.0;
const ROUGH_MAX_M: f64 = 1.5;
const ROUGH_FADE_M: f64 = 3.0;
const ROUGH_SLOPE_SHARE: f64 = 0.6;
/// Scale of the noise that breaks crevasses up across flow and patches their relief.
const PATCH_M: f64 = 40.0;
/// Margin around the lakes searched for the glacier's catchment.
const WINDOW_M: f64 = 1000.0;
/// Ice cells the pit fill may raise (a failing check above it).
pub(crate) const FILLED_MAX_SHARE: f64 = 0.01;

/// The judgment values, for the cache tag and the report.
pub(crate) fn params() -> Value {
    json!({"version": VERSION, "yield_kpa": YIELD_KPA, "tongue_m": TONGUE_M,
           "envelope_m": ENVELOPE_M, "ice_min_m": ICE_MIN_M, "crevasse_m": CREVASSE_M,
           "rise": [RISE_PER_M, RISE_MIN_M, RISE_MAX_M],
           "rough_max_m": ROUGH_MAX_M, "rough_fade_m": ROUGH_FADE_M,
           "rough_slope_share": ROUGH_SLOPE_SHARE, "patch_m": PATCH_M, "window_m": WINDOW_M})
}

/// The layout's snowline grid (`layout.json` + `snowline.f32`, rows north to south, its
/// north-west corner at (0, extent) in the layout plane).
pub(crate) struct Snowline {
    w: usize,
    h: usize,
    cell_m: f64,
    values: Vec<f32>,
    pub sha256: String,
}

impl Snowline {
    pub fn open(dir: &Path) -> Result<Self> {
        let m: Value = serde_json::from_slice(&io_context(
            format!("read {}", dir.join("layout.json").display()),
            std::fs::read(dir.join("layout.json")),
        )?)?;
        ensure(
            m["kind"] == "world_layout_l0",
            format!("{} is not an L0 layout", dir.display()),
        )?;
        let (w, h) = (
            m["size"][0].as_u64().unwrap_or(0) as usize,
            m["size"][1].as_u64().unwrap_or(0) as usize,
        );
        let cell_m = m["cell_m"].as_f64().unwrap_or(0.0);
        ensure(w > 0 && h > 0 && cell_m > 0.0, "layout.json size or cell_m")?;
        let path = dir.join("snowline.f32");
        let bytes = io_context(format!("read {}", path.display()), std::fs::read(&path))?;
        ensure(bytes.len() == w * h * 4, "snowline.f32 size")?;
        Ok(Self {
            w,
            h,
            cell_m,
            values: bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
            sha256: crate::acquisition::hash_file(&path)?.0,
        })
    }
    /// Bilinear at (east, north) metres of the layout plane, clamped to the grid.
    pub fn at(&self, east: f64, north: f64) -> f32 {
        let fx = (east / self.cell_m - 0.5).clamp(0.0, (self.w - 1) as f64);
        let fy = ((self.h as f64 * self.cell_m - north) / self.cell_m - 0.5)
            .clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let v = |x: usize, y: usize| f64::from(self.values[y * self.w + x]);
        let top = v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx;
        let bottom = v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx;
        (top * (1.0 - ty) + bottom * ty) as f32
    }
}

/// An ice lake at or above the snowline.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub id: u32,
    pub level: f32,
    /// Its spill cell (world index); None for a closed lake (its col is searched).
    pub exit: Option<u64>,
    /// [x0, y0, x1, y1] inclusive, and its cells.
    pub bbox: [u64; 4],
    pub cells: u64,
    pub snowline: f32,
}

pub(crate) struct Inputs<'a> {
    pub world: World,
    pub spacing_m: f64,
    /// m3's routing on the ground before ice (low 4 bits: D8 receiver).
    pub flow: &'a Layer<u8>,
    /// Lakes, ice and sea by id: level and spill cell, for the lakes a glacier must not dam.
    pub lakes: &'a BTreeMap<u32, (f32, Option<u64>)>,
    /// River body ids: ice covers them.
    pub rivers: &'a BTreeSet<u32>,
}

/// The rasters the glaciers are written into, initially copies of the ground, the water body
/// and surface, and an empty mask.
pub(crate) struct Outputs {
    pub ground: Layer<f32>,
    pub body: Layer<u32>,
    pub surface: Layer<f32>,
    pub mask: Layer<u8>,
    /// Cells a built glacier owns (its catchment and the ring around it): later glaciers
    /// leave them as they are, so each glacier's drainage stays valid.
    pub claimed: Layer<u8>,
}

/// A built (or skipped) glacier: its lakes (the lowest first) and figures.
#[derive(Clone, Debug)]
pub(crate) struct Built {
    pub ids: Vec<u32>,
    pub skipped: Option<String>,
    pub figures: Value,
    pub ice_cells: u64,
    pub filled_ice: u64,
    /// Lakes a glacier's catchment borders (their m3 level is checked).
    pub guarded: Vec<u32>,
}

const UNK: u8 = 0;
const IN: u8 = 1;
const OUT: u8 = 2;
const POND: u8 = 3;
const WALK: u8 = 9;

/// f64 ordered for heaps.
#[derive(Clone, Copy, PartialEq)]
struct Key(f64);
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// One window's cells, read from the outputs (earlier glaciers included).
struct Win {
    world: World,
    r: Rect,
    z: Vec<f64>,
    body: Vec<u32>,
    surf: Vec<f32>,
    flow: Vec<u8>,
    mask: Vec<u8>,
    claimed: Vec<u8>,
}

impl Win {
    fn read(inp: &Inputs, out: &Outputs, r: Rect) -> Result<Self> {
        Ok(Self {
            world: inp.world,
            r,
            z: out.ground.read(r)?.into_iter().map(f64::from).collect(),
            body: out.body.read(r)?,
            surf: out.surface.read(r)?,
            flow: inp.flow.read(r)?.into_iter().map(|f| f & 15).collect(),
            mask: out.mask.read(r)?,
            claimed: out.claimed.read(r)?,
        })
    }
    fn local(&self, world: World, i: u64) -> Option<usize> {
        let (x, y) = world.xy(i);
        self.r.local(x, y)
    }
    /// Neighbours of `j` inside the window: (local index, step in cells).
    fn around(&self, j: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let (w, h) = (self.r.w as i64, self.r.h as i64);
        let (x, y) = ((j % self.r.w) as i64, (j / self.r.w) as i64);
        OFFSETS
            .iter()
            .enumerate()
            .filter_map(move |(k, &(dy, dx))| {
                let (nx, ny) = (x + dx, y + dy);
                (nx >= 0 && ny >= 0 && nx < w && ny < h)
                    .then(|| ((ny * w + nx) as usize, step_len(k as u8 + 1)))
            })
    }
    fn world_edge(&self, j: usize) -> bool {
        let (x, y) = self.r.world_xy(j);
        self.world.is_edge(x, y)
    }
    /// Neighbours of `j` in a 16-cell stencil (8 plus knight moves), for path lengths with
    /// less grid bias: (local index, step in cells).
    fn around16(&self, j: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        const KNIGHT: [(i64, i64); 8] = [
            (-2, -1),
            (-2, 1),
            (2, -1),
            (2, 1),
            (-1, -2),
            (-1, 2),
            (1, -2),
            (1, 2),
        ];
        let (w, h) = (self.r.w as i64, self.r.h as i64);
        let (x, y) = ((j % self.r.w) as i64, (j / self.r.w) as i64);
        self.around(j)
            .chain(KNIGHT.iter().filter_map(move |&(dy, dx)| {
                let (nx, ny) = (x + dx, y + dy);
                (nx >= 0 && ny >= 0 && nx < w && ny < h)
                    .then(|| ((ny * w + nx) as usize, 5f64.sqrt()))
            }))
    }
    fn on_ring(&self, j: usize) -> bool {
        let (x, y) = (j % self.r.w, j / self.r.w);
        x == 0 || y == 0 || x + 1 == self.r.w || y + 1 == self.r.h
    }
    /// The receiver of `j`: Some(Some(local)), Some(None) leaving the window, None for none.
    fn receiver(&self, j: usize) -> Option<Option<usize>> {
        let d = self.flow[j];
        if d == 0 {
            return None;
        }
        let (dy, dx) = OFFSETS[d as usize - 1];
        let (x, y) = ((j % self.r.w) as i64 + dx, (j / self.r.w) as i64 + dy);
        Some(
            (x >= 0 && y >= 0 && x < self.r.w as i64 && y < self.r.h as i64)
                .then(|| y as usize * self.r.w + x as usize),
        )
    }
    /// Height water at `j` drains at: its surface where wet.
    fn top(&self, j: usize) -> f64 {
        if self.surf[j].is_finite() {
            self.z[j].max(f64::from(self.surf[j]))
        } else {
            self.z[j]
        }
    }
}

/// Window around the lakes' boxes, grown by WINDOW_M.
fn window(world: World, spacing_m: f64, boxes: impl Iterator<Item = [u64; 4]>) -> Rect {
    let m = (WINDOW_M / spacing_m).ceil() as u64;
    let mut b = [u64::MAX, u64::MAX, 0, 0];
    for x in boxes {
        b = [
            b[0].min(x[0]),
            b[1].min(x[1]),
            b[2].max(x[2]),
            b[3].max(x[3]),
        ];
    }
    let (x0, y0) = (b[0].saturating_sub(m), b[1].saturating_sub(m));
    let (x1, y1) = ((b[2] + m).min(world.w - 1), (b[3] + m).min(world.h - 1));
    Rect {
        x0,
        y0,
        w: (x1 - x0 + 1) as usize,
        h: (y1 - y0 + 1) as usize,
    }
}

/// The col of a closed lake: [col, first cell beyond it], the lowest pass out of its basin
/// on the ground (a priority flood from its cells); None if not inside the window.
fn col(win: &Win, id: u32) -> Option<[usize; 2]> {
    let n = win.z.len();
    let mut seen: Vec<bool> = win.body.iter().map(|&b| b == id).collect();
    let mut from = vec![u32::MAX; n];
    let mut heap: BinaryHeap<_> = (0..n)
        .filter(|&j| seen[j])
        .map(|j| Reverse((Key(win.z[j]), j)))
        .collect();
    let mut level = f64::NEG_INFINITY;
    while let Some(Reverse((Key(h), j))) = heap.pop() {
        if h < level {
            return Some([from[j] as usize, j]);
        }
        level = level.max(h);
        if win.on_ring(j) {
            return None;
        }
        for (k, _) in win.around(j) {
            if !seen[k] {
                seen[k] = true;
                from[k] = j as u32;
                heap.push(Reverse((Key(win.z[k]), k)));
            }
        }
    }
    None
}

/// Where each cell's receiver chain ends: IN (a target), OUT, or POND (a pit cell `pit`).
fn resolve(win: &Win, target: &[bool], stop: &[bool]) -> (Vec<u8>, Vec<u32>) {
    let n = win.z.len();
    let (mut state, mut pit) = (vec![UNK; n], vec![u32::MAX; n]);
    let mut path = Vec::new();
    for start in 0..n {
        if state[start] != UNK {
            continue;
        }
        let mut cur = start;
        let (end, p) = loop {
            match state[cur] {
                UNK => {}
                WALK => break (OUT, u32::MAX), // a cycle: refused by the model, kept out here
                s => break (s, pit[cur]),
            }
            if target[cur] {
                break (IN, u32::MAX);
            }
            if stop[cur] {
                break (OUT, u32::MAX);
            }
            match win.receiver(cur) {
                // The world's edge drains out of it.
                None if win.world_edge(cur) => break (OUT, u32::MAX),
                None => break (POND, cur as u32),
                Some(None) => break (OUT, u32::MAX),
                Some(Some(next)) => {
                    state[cur] = WALK;
                    path.push(cur);
                    cur = next;
                }
            }
        };
        if state[cur] == UNK {
            path.push(cur);
        }
        for &c in &path {
            (state[c], pit[c]) = (end, p);
        }
        path.clear();
    }
    (state, pit)
}

/// Value noise in [0, 1] at (x, y) metres, lattice `cell_m`, seeded.
fn noise(x: f64, y: f64, cell_m: f64, seed: u64) -> f64 {
    let hash = |ix: i64, iy: i64| {
        let mut v = (ix as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_add(seed.wrapping_mul(0x1656_67B1_9E37_79F9));
        v ^= v >> 30;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v ^= v >> 27;
        v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
        v ^= v >> 31;
        (v >> 11) as f64 / (1u64 << 53) as f64
    };
    let (fx, fy) = (x / cell_m, y / cell_m);
    let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (s(fx - ix as f64), s(fy - iy as f64));
    let top = hash(ix, iy) * (1.0 - tx) + hash(ix + 1, iy) * tx;
    let bottom = hash(ix, iy + 1) * (1.0 - tx) + hash(ix + 1, iy + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// The crevasse profile over one spacing: a sharp rise (the downstream step) and a long fall.
fn profile(u: f64) -> f64 {
    let f = u - u.floor();
    if f < 0.2 {
        -0.5 + f / 0.2
    } else {
        0.5 - (f - 0.2) / 0.8
    }
}

/// Builds every glacier (the lowest lakes first; a lake whose outlet drains into a glacier's
/// catchment joins it), writing into `out`.
pub(crate) fn build(inp: &Inputs, cands: &[Candidate], out: &Outputs) -> Result<Vec<Built>> {
    let mut order: Vec<&Candidate> = cands.iter().collect();
    order.sort_by(|a, b| a.level.total_cmp(&b.level).then(a.id.cmp(&b.id)));
    let (mut done, mut glaciers) = (BTreeSet::new(), BTreeSet::new());
    let mut built = Vec::new();
    for seed in &order {
        if done.contains(&seed.id) {
            continue;
        }
        let mut group = vec![(*seed).clone()];
        let b = loop {
            let b = one(inp, &group, cands, out, &done, &glaciers)?;
            match b {
                Grown::Join(more) => group.extend(more),
                Grown::Done(b) => break b,
            }
        };
        done.extend(group.iter().map(|c| c.id));
        if b.skipped.is_none() {
            glaciers.extend(group.iter().map(|c| c.id));
        }
        built.push(b);
    }
    Ok(built)
}

/// A surface label: (S, cell, source height, path length, allowed root, parent, raw
/// plastic height before flattening).
type Label = (Key, usize, Key, Key, bool, u32, Key);

enum Grown {
    Join(Vec<Candidate>),
    Done(Built),
}

/// One glacier from `group` (its first lake the lowest): either more lakes that join it, or
/// the built (or skipped) glacier, written into `out`.
fn one(
    inp: &Inputs,
    group: &[Candidate],
    all: &[Candidate],
    out: &Outputs,
    done: &BTreeSet<u32>,
    glaciers: &BTreeSet<u32>,
) -> Result<Grown> {
    let world = inp.world;
    let sp = inp.spacing_m;
    let ids: Vec<u32> = group.iter().map(|c| c.id).collect();
    let skip = |why: String| {
        Ok(Grown::Done(Built {
            ids: ids.clone(),
            skipped: Some(why),
            figures: Value::Null,
            ice_cells: 0,
            filled_ice: 0,
            guarded: Vec::new(),
        }))
    };
    let r = window(world, sp, group.iter().map(|c| c.bbox));
    let win = Win::read(inp, out, r)?;
    let n = win.z.len();
    let member = |b: u32| ids.contains(&b);
    // The exit path: [exit, (closed: beyond the col)], then the receivers for TONGUE_M.
    let seed = &group[0];
    let start = match seed.exit {
        Some(e) => match win.local(world, e) {
            Some(j) => vec![j],
            None => return skip("exit outside its window".into()),
        },
        None => match col(&win, seed.id) {
            Some([c, b]) => vec![c, b],
            None => return skip("no spill col inside its window".into()),
        },
    };
    let foreign = |j: usize| {
        let b = win.body[j];
        (b != 0 && !member(b) && !inp.rivers.contains(&b)) || win.claimed[j] != 0
    };
    let mut path = vec![start[0]];
    let mut length = 0.0;
    let mut cur = start[0];
    let mut forced = start.get(1).copied();
    let terminus = loop {
        let next = match forced.take() {
            Some(b) => Some(Some(b)),
            None => win.receiver(cur),
        };
        let next = match next {
            Some(Some(nx)) => nx,
            _ => break path.pop().expect("path"),
        };
        if foreign(next) || member(win.body[next]) {
            break path.pop().expect("path");
        }
        let (a, b) = (win.r.world_xy(cur), win.r.world_xy(next));
        length += if a.0 != b.0 && a.1 != b.1 {
            std::f64::consts::SQRT_2
        } else {
            1.0
        } * sp;
        if length >= TONGUE_M {
            break next;
        }
        path.push(next);
        cur = next;
    };
    let mut target: Vec<bool> = win.body.iter().map(|&b| member(b)).collect();
    for &j in &path {
        target[j] = true;
    }
    let mut stop = vec![false; n];
    for j in 0..n {
        stop[j] = !target[j] && foreign(j);
    }
    stop[terminus] = true;
    target[terminus] = false;
    let (mut state, pit) = resolve(&win, &target, &stop);
    // Lakes whose outlet drains into this catchment join the glacier.
    let joins: Vec<Candidate> = all
        .iter()
        .filter(|c| !member(c.id) && !done.contains(&c.id))
        .filter(|c| {
            let e = match c.exit {
                Some(e) => win.local(world, e),
                // A closed lake inside the window joins through its col.
                None if win.body.contains(&c.id) => col(&win, c.id).map(|[k, _]| k),
                None => None,
            };
            e.is_some_and(|e| state[e] == IN)
        })
        .cloned()
        .collect();
    if !joins.is_empty() {
        return Ok(Grown::Join(joins));
    }
    let lake_cells: u64 = group.iter().map(|c| c.cells).sum();
    let k_m = 2.0 * YIELD_KPA * 1e3 / (ICE_DENSITY * GRAVITY);
    let zt = win.top(terminus);
    // The surface flattens toward `ceiling`, `rise` above the seed's spill (its exit, or a
    // closed lake's col), high enough to cover every joined lake.
    let base = zt.max(win.top(start[0]));
    let span = group
        .iter()
        .map(|c| f64::from(c.level) - base)
        .fold(0.0, f64::max);
    let rise = (RISE_PER_M * (lake_cells as f64 * sp * sp).sqrt())
        .clamp(RISE_MIN_M, RISE_MAX_M)
        .max(1.5 * (span + ENVELOPE_M));
    let ceiling = base + rise;
    // A label's surface from its raw plastic height: flattening above max(base, source).
    let flatten = |zs: f64, v: f64| {
        let b = base.max(zs);
        let room = (ceiling - b).max(ICE_MIN_M);
        if v <= b {
            v
        } else {
            b + room * ((v - b) / room).tanh()
        }
    };
    // The pit fill reaches the joined lakes' levels too: an upper lake of a chain above the
    // ceiling fills flat to its spill rather than staying a pit.
    let fill_cap = group
        .iter()
        .map(|c| f64::from(c.level))
        .fold(ceiling, f64::max);
    // Pits draining into the catchment between the terminus and the ceiling join it (they
    // fill with ice).
    for _ in 0..8 {
        let mut pits = BTreeSet::new();
        for j in 0..n {
            if state[j] != POND {
                continue;
            }
            let low = win.z[pit[j] as usize];
            if low < ceiling && low > zt && win.around(j).any(|(k, _)| state[k] == IN) {
                pits.insert(pit[j]);
            }
        }
        if pits.is_empty() {
            break;
        }
        for j in 0..n {
            if state[j] == POND && pits.contains(&pit[j]) {
                state[j] = IN;
            }
        }
    }
    let omega: Vec<bool> = state.iter().map(|&s| s == IN).collect();
    // The terminus and its receivers below it: sources draining there keep the catchment.
    let mut below = vec![false; n];
    {
        let mut c = terminus;
        let mut steps = 0;
        loop {
            below[c] = true;
            steps += 1;
            match win.receiver(c) {
                Some(Some(nx)) if !below[nx] && steps < n => c = nx,
                _ => break,
            }
        }
    }
    let mut joins_below = vec![UNK; n];
    let mut reaches_below = |j: usize| -> bool {
        let mut walk = Vec::new();
        let mut c = j;
        let v = loop {
            if joins_below[c] != UNK {
                break joins_below[c] == IN;
            }
            if below[c] {
                break true;
            }
            if omega[c] || walk.len() > n {
                break false;
            }
            walk.push(c);
            match win.receiver(c) {
                Some(Some(nx)) => c = nx,
                _ => break false,
            }
        };
        for w in walk {
            joins_below[w] = if v { IN } else { OUT };
        }
        v
    };
    // The plastic surface (perfectly plastic ice: slope × thickness = τ/ρg), label-setting
    // from the sources (T and the cells around the catchment), each step solving
    // (S − S_parent)(S − z) = (τ/ρg)·step. Over a flat bed this is Nye's √(2τ/ρg · d); up a
    // wall the ice thins to (τ/ρg)/slope. A label carries its source height and path length:
    // the ice stays under the Nye envelope from its source plus ENVELOPE_M.
    let a_m = k_m / 2.0;
    let step_up = |sp_: f64, z: f64, l: f64| {
        0.5 * ((sp_ + z) + ((sp_ - z) * (sp_ - z) + 4.0 * a_m * l).sqrt())
    };
    let mut best = vec![f64::INFINITY; n];
    let mut s_at = vec![f64::NAN; n];
    let mut parent = vec![u32::MAX; n];
    let mut allowed = vec![false; n];
    let mut ice = vec![false; n];
    let mut fixed = vec![false; n];
    let mut heap: BinaryHeap<Reverse<Label>> = BinaryHeap::new();
    for j in 0..n {
        let src = j == terminus || (!omega[j] && win.around(j).any(|(k, _)| omega[k]));
        if src && win.top(j) < ceiling {
            let a = j == terminus || reaches_below(j);
            let h = win.top(j);
            heap.push(Reverse((Key(h), j, Key(h), Key(0.0), a, u32::MAX, Key(h))));
        }
    }
    while let Some(Reverse((Key(s), j, Key(zs), Key(d), a, p, Key(raw)))) = heap.pop() {
        if fixed[j] {
            continue;
        }
        fixed[j] = true;
        let is_source = !omega[j];
        if !is_source {
            ice[j] = true;
        }
        s_at[j] = s;
        parent[j] = p;
        allowed[j] = a;
        for (k, step) in win.around16(j) {
            // Ice flows down to a source, never up from it.
            if !omega[k] || fixed[k] || (is_source && win.z[k] < s) {
                continue;
            }
            let dd = d + step * sp;
            let rk = step_up(raw, win.z[k], step * sp);
            let sk = flatten(zs, rk);
            let ok = sk - win.z[k] >= ICE_MIN_M && rk <= zs + (k_m * dd).sqrt() + ENVELOPE_M;
            if ok && sk < best[k] {
                best[k] = sk;
                heap.push(Reverse((
                    Key(sk),
                    k,
                    Key(zs),
                    Key(dd),
                    a,
                    j as u32,
                    Key(rk),
                )));
            }
        }
    }
    drop(heap);
    if (0..n).any(|j| ice[j] && win.on_ring(j)) {
        return skip("ice reaches the window edge".into());
    }
    // Crevasse phase: path length from the terminus through the ice.
    let mut dist = vec![f64::INFINITY; n];
    let mut h2: BinaryHeap<Reverse<(Key, usize)>> = BinaryHeap::new();
    dist[terminus] = 0.0;
    h2.push(Reverse((Key(0.0), terminus)));
    while let Some(Reverse((Key(d), j))) = h2.pop() {
        if d > dist[j] {
            continue;
        }
        for (k, step) in win.around16(j) {
            let dk = d + step * sp;
            if ice[k] && dk < dist[k] {
                dist[k] = dk;
                h2.push(Reverse((Key(dk), k)));
            }
        }
    }
    let seed_noise = u64::from(seed.id);
    let mut elev = win.z.clone();
    for j in 0..n {
        if !ice[j] {
            continue;
        }
        let s = s_at[j];
        let p = parent[j] as usize;
        let ps = if parent[j] == u32::MAX { s } else { s_at[p] };
        let (jx, jy) = win.r.world_xy(j);
        let step = if parent[j] == u32::MAX {
            1.0
        } else {
            let (px, py) = win.r.world_xy(p);
            (px as f64 - jx as f64).hypot(py as f64 - jy as f64)
        } * sp;
        let slope = ((s - ps) / step).max(0.0);
        let (xm, ym) = (jx as f64 * sp, jy as f64 * sp);
        let thick = s - win.z[j];
        let t = (thick / ROUGH_FADE_M).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        // Crevasse fields in patches, not everywhere.
        let u = ((noise(xm, ym, PATCH_M, seed_noise) - 0.4) / 0.3).clamp(0.0, 1.0);
        let patch = u * u * (3.0 - 2.0 * u);
        let a = (ROUGH_SLOPE_SHARE * slope * CREVASSE_M).min(ROUGH_MAX_M) * fade * patch;
        let rough = if dist[j].is_finite() {
            let phase = noise(xm, ym, PATCH_M / 2.0, seed_noise ^ 0xA5A5);
            a * profile(dist[j] / CREVASSE_M + phase)
        } else {
            0.0
        };
        elev[j] = win.z[j].max(s + rough);
    }
    // Pit fill over the catchment from all around it: no cell of it holds water.
    let before: Vec<bool> = ice.clone();
    let mut seen = vec![false; n];
    let mut h3: BinaryHeap<Reverse<(Key, usize)>> = BinaryHeap::new();
    for j in 0..n {
        let seed_cell =
            (!omega[j] && win.around(j).any(|(k, _)| omega[k])) || (omega[j] && win.on_ring(j));
        if seed_cell {
            seen[j] = true;
            h3.push(Reverse((
                Key(if omega[j] { elev[j] } else { win.top(j) }),
                j,
            )));
        }
    }
    let (mut filled_ice, mut filled_other) = (0u64, 0u64);
    while let Some(Reverse((Key(h), j))) = h3.pop() {
        for (k, _) in win.around(j) {
            if omega[k] && !seen[k] {
                seen[k] = true;
                if elev[k] < h {
                    // A pit off the ice whose spill lies above the fill cap keeps no ice: it
                    // stays as it was (a whole depression shares its spill).
                    if before[k] || h <= fill_cap + 1e-3 {
                        elev[k] = h;
                        if before[k] {
                            filled_ice += 1;
                        } else {
                            filled_other += 1;
                            ice[k] = true;
                        }
                    }
                    h3.push(Reverse((Key(h), k)));
                } else {
                    h3.push(Reverse((Key(elev[k]), k)));
                }
            }
        }
    }
    // Lakes draining into the catchment keep their spill: no ice above their level on
    // their outlet path.
    let mut guarded = Vec::new();
    for (&id, &(level, exit)) in inp.lakes {
        if member(id) || glaciers.contains(&id) {
            continue;
        }
        let Some(e) = exit.and_then(|e| win.local(world, e)) else {
            continue;
        };
        if !omega[e] {
            continue;
        }
        guarded.push(id);
        let mut c = e;
        let mut steps = 0;
        loop {
            if elev[c] > f64::from(level) && elev[c] > win.z[c] {
                return skip(format!("would dam lake {id}"));
            }
            steps += 1;
            match win.receiver(c) {
                Some(Some(nx)) if omega[nx] && steps < n => c = nx,
                _ => break,
            }
        }
    }
    // Write: ice cells take the new height and lose their water; the lakes lose all of it.
    let mut body = win.body.clone();
    let mut surf = win.surf.clone();
    let mut mask = win.mask.clone();
    let mut ground = Vec::with_capacity(n);
    let (mut cells, mut thick_sum, mut thick_max, mut top, mut captured) =
        (0u64, 0.0f64, 0.0f64, f64::NEG_INFINITY, 0u64);
    let mut uncovered = 0u64;
    let mut rivers_covered = 0u64;
    let mut bbox = [u64::MAX, u64::MAX, 0, 0];
    for j in 0..n {
        let lake = member(win.body[j]);
        if ice[j] {
            let (x, y) = win.r.world_xy(j);
            bbox = [
                bbox[0].min(x),
                bbox[1].min(y),
                bbox[2].max(x),
                bbox[3].max(y),
            ];
            cells += 1;
            let t = elev[j] - win.z[j];
            thick_sum += t;
            thick_max = thick_max.max(t);
            top = top.max(elev[j]);
            captured += u64::from(!allowed[j] && before[j]);
            if win.body[j] != 0 && !lake {
                rivers_covered += 1;
            }
            mask[j] = 255;
            body[j] = 0;
            surf[j] = f32::NAN;
        } else if lake {
            uncovered += 1;
            body[j] = 0;
            surf[j] = f32::NAN;
        }
        ground.push(if ice[j] {
            elev[j] as f32
        } else {
            win.z[j] as f32
        });
    }
    out.ground.write(r, &ground)?;
    out.body.write(r, &body)?;
    out.surface.write(r, &surf)?;
    out.mask.write(r, &mask)?;
    let mut claimed = win.claimed.clone();
    for j in 0..n {
        if omega[j] {
            claimed[j] = 1;
            for (k, _) in win.around(j) {
                claimed[k] = 1;
            }
        }
    }
    out.claimed.write(r, &claimed)?;
    let km2 = sp * sp / 1e6;
    let round = |v: f64, k: f64| (v * k).round() / k;
    let tw = win.r.world_xy(terminus);
    let figures = json!({
        "lakes": ids,
        "terminus_cell": [tw.0, tw.1],
        "ceiling_m": round(ceiling, 100.0),
        "rise_m": round(rise, 100.0),
        "lake_km2": round(lake_cells as f64 * km2, 1e6),
        "area_km2": round(cells as f64 * km2, 1e6),
        "max_thickness_m": round(thick_max, 100.0),
        "mean_thickness_m": round(if cells > 0 { thick_sum / cells as f64 } else { 0.0 }, 100.0),
        "surface_max_m": round(top, 100.0),
        "filled_ice_cells": filled_ice,
        "filled_pit_cells": filled_other,
        "captured_km2": round(captured as f64 * km2, 1e6),
        "uncovered_lake_cells": uncovered,
        "river_cells_covered": rivers_covered,
        // [x0, y0, x1, y1) in cells, as the exporter reads it.
        "bbox": [bbox[0], bbox[1], bbox[2] + 1, bbox[3] + 1],
        "guarded": guarded,
    });
    Ok(Grown::Done(Built {
        ids,
        skipped: None,
        figures,
        ice_cells: cells,
        filled_ice,
        guarded,
    }))
}

/// Ice lakes at or above the snowline at their centroid: the glacier candidates. `extents`
/// from `ice::frozen`, `centre` a cell centre in the layout plane.
pub(crate) fn select(
    bodies: &Value,
    ice: &BTreeSet<u32>,
    extents: &BTreeMap<u32, super::ice::Extent>,
    snowline: &Snowline,
    world: World,
    centre: impl Fn(f64, f64) -> (f64, f64),
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for b in bodies["bodies"].as_array().into_iter().flatten() {
        let Some(id) = b["id"].as_u64().and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        let (Some(e), Some(level)) = (extents.get(&id), b["level_m"].as_f64()) else {
            continue;
        };
        if !ice.contains(&id) || b["kind"] != "lake" {
            continue;
        }
        let (cx, cy) = e.centroid();
        let (east, north) = centre(cx, cy);
        let line = snowline.at(east, north);
        if (level as f32) < line {
            continue;
        }
        let exit = b["exit_cell"].as_array().and_then(|a| {
            let (x, y) = (a.first()?.as_u64()?, a.get(1)?.as_u64()?);
            (x < world.w && y < world.h).then(|| world.index(x, y))
        });
        out.push(Candidate {
            id,
            level: level as f32,
            exit,
            bbox: e.bbox,
            cells: e.cells,
            snowline: line,
        });
    }
    out
}

/// bodies.json after the glaciers: each built glacier's lakes become kind `glacier` (no
/// `level_m`: their former level is `lake_level_m`, their area `lake_area_km2`); the lowest
/// carries the glacier's figures, the others name it in `glacier`.
pub(crate) fn relabel(bodies: &mut Value, built: &[Built], cands: &[Candidate]) {
    let line: BTreeMap<u32, f32> = cands.iter().map(|c| (c.id, c.snowline)).collect();
    for g in built.iter().filter(|g| g.skipped.is_none()) {
        for b in bodies["bodies"].as_array_mut().into_iter().flatten() {
            let Some(id) = b["id"].as_u64().and_then(|v| u32::try_from(v).ok()) else {
                continue;
            };
            if !g.ids.contains(&id) {
                continue;
            }
            let o = b.as_object_mut().expect("body object");
            let level = o.remove("level_m").unwrap_or(Value::Null);
            let area = o.remove("area_km2").unwrap_or(Value::Null);
            o.insert("kind".into(), "glacier".into());
            o.insert("lake_level_m".into(), level);
            o.insert("lake_area_km2".into(), area);
            o.insert("glacier".into(), json!(g.ids[0]));
            o.insert(
                "snowline_m".into(),
                json!(line.get(&id).map(|v| (f64::from(*v) * 10.0).round() / 10.0)),
            );
            if id == g.ids[0] {
                for (k, v) in g.figures.as_object().into_iter().flatten() {
                    if k != "lakes" {
                        o.insert(k.clone(), v.clone());
                    }
                }
            } else {
                o.insert("area_km2".into(), json!(0.0));
            }
        }
    }
}

/// A file of the world's size full of zeros (sparse).
pub(crate) fn zeros(path: &Path, bytes: u64) -> Result<()> {
    let f = io_context(
        format!("create {}", path.display()),
        std::fs::File::create(path),
    )?;
    io_context("size mask", f.set_len(bytes))
}

impl Built {
    fn to_json(&self) -> Value {
        json!({"ids": self.ids, "skipped": self.skipped, "figures": self.figures,
               "ice_cells": self.ice_cells, "filled_ice": self.filled_ice, "guarded": self.guarded})
    }
    fn from_json(v: &Value) -> Option<Self> {
        let ids = |v: &Value| -> Option<Vec<u32>> {
            v.as_array()?
                .iter()
                .map(|x| x.as_u64().and_then(|x| u32::try_from(x).ok()))
                .collect()
        };
        Some(Self {
            ids: ids(&v["ids"])?,
            skipped: v["skipped"].as_str().map(str::to_owned),
            figures: v["figures"].clone(),
            ice_cells: v["ice_cells"].as_u64()?,
            filled_ice: v["filled_ice"].as_u64()?,
            guarded: ids(&v["guarded"])?,
        })
    }
}

/// The glaciers in the work directory's `dir` (ground.f32, body.u32, surface.f32, mask.u8,
/// glaciers.json), built from copies of `ground`, `body` and `surface` unless already there.
pub(crate) fn cached(
    dir: &Path,
    ground: &Path,
    body: &Path,
    surface: &Path,
    inp: &Inputs,
    cands: &[Candidate],
) -> Result<Vec<Built>> {
    let list = dir.join("glaciers.json");
    if !list.exists() {
        let tmp = dir.with_extension("partial");
        if tmp.exists() {
            io_context("clear a partial glacier run", std::fs::remove_dir_all(&tmp))?;
        }
        io_context(
            "create the glacier directory",
            std::fs::create_dir_all(&tmp),
        )?;
        for (src, name) in [
            (ground, "ground.f32"),
            (body, "body.u32"),
            (surface, "surface.f32"),
        ] {
            let done = std::process::Command::new("cp")
                .arg("--reflink=auto")
                .arg(src)
                .arg(tmp.join(name))
                .status();
            ensure(
                io_context(format!("copy {}", src.display()), done)?.success(),
                format!("copy {} failed", src.display()),
            )?;
        }
        let w = inp.world;
        zeros(&tmp.join("mask.u8"), w.w * w.h)?;
        zeros(&tmp.join("claimed.u8"), w.w * w.h)?;
        let out = Outputs {
            ground: Layer::open(w, &tmp.join("ground.f32"))?,
            body: Layer::open(w, &tmp.join("body.u32"))?,
            surface: Layer::open(w, &tmp.join("surface.f32"))?,
            mask: Layer::open(w, &tmp.join("mask.u8"))?,
            claimed: Layer::open(w, &tmp.join("claimed.u8"))?,
        };
        let built = build(inp, cands, &out)?;
        let v: Vec<Value> = built.iter().map(Built::to_json).collect();
        io_context(
            "write glaciers.json",
            std::fs::write(tmp.join("glaciers.json"), serde_json::to_vec_pretty(&v)?),
        )?;
        io_context("publish the glaciers", std::fs::rename(&tmp, dir))?;
    }
    let v: Value = serde_json::from_slice(&io_context(
        format!("read {}", list.display()),
        std::fs::read(&list),
    )?)?;
    v.as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            Built::from_json(b).ok_or_else(|| {
                crate::acquisition::AcquisitionError::Invalid("glaciers.json entry".into())
            })
        })
        .collect()
}

#[cfg(test)]
impl Snowline {
    /// One snowline height everywhere (a 1 x 1 layout of `extent_m`).
    fn flat(level: f32, extent_m: f64) -> Self {
        Self {
            w: 1,
            h: 1,
            cell_m: extent_m,
            values: vec![level],
            sha256: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A test world: ground, lakes flooded from seed cells (id, level, seed), D8 steepest
    /// descent elsewhere (lake cells none; an exit drains away from its lake).
    struct T {
        world: World,
        z: Vec<f32>,
        body: Vec<u32>,
        surf: Vec<f32>,
        flow: Vec<u8>,
        exits: BTreeMap<u32, Option<u64>>,
        levels: BTreeMap<u32, f32>,
    }

    fn make(w: u64, h: u64, z: impl Fn(u64, u64) -> f32, lakes: &[(u32, f32, (u64, u64))]) -> T {
        let world = World { w, h };
        let n = (w * h) as usize;
        let z: Vec<f32> = (0..n as u64).map(|i| z(i % w, i / w)).collect();
        let mut body = vec![0u32; n];
        let mut surf = vec![f32::NAN; n];
        let mut exits = BTreeMap::new();
        let mut levels = BTreeMap::new();
        for &(id, level, (sx, sy)) in lakes {
            let mut stack = vec![world.index(sx, sy)];
            while let Some(c) = stack.pop() {
                if body[c as usize] != 0 || z[c as usize] >= level {
                    continue;
                }
                body[c as usize] = id;
                surf[c as usize] = level;
                let (x, y) = world.xy(c);
                for k in 0..8 {
                    if let Some((nx, ny)) = world.neighbour(x, y, k) {
                        stack.push(world.index(nx, ny));
                    }
                }
            }
            levels.insert(id, level);
        }
        let steepest = |c: u64, skip: &dyn Fn(u64) -> bool| -> u8 {
            let (x, y) = world.xy(c);
            let mut best = (0.0f32, 0u8);
            for k in 0..8 {
                if let Some((nx, ny)) = world.neighbour(x, y, k) {
                    let o = world.index(nx, ny);
                    if skip(o) {
                        continue;
                    }
                    let drop = (z[c as usize] - z[o as usize]) / step_len(k as u8 + 1) as f32;
                    if drop > best.0 {
                        best = (drop, k as u8 + 1);
                    }
                }
            }
            best.1
        };
        let mut flow: Vec<u8> = (0..n as u64)
            .map(|c| {
                if body[c as usize] != 0 {
                    0
                } else {
                    steepest(c, &|_| false)
                }
            })
            .collect();
        for &(id, level, _) in lakes {
            // The exit: the lowest non-lake neighbour of the lake, draining away from it.
            let mut exit: Option<(f32, u64)> = None;
            for c in 0..n as u64 {
                if body[c as usize] != id {
                    continue;
                }
                let (x, y) = world.xy(c);
                for k in 0..8 {
                    if let Some((nx, ny)) = world.neighbour(x, y, k) {
                        let o = world.index(nx, ny);
                        if body[o as usize] == 0 && exit.is_none_or(|e| z[o as usize] < e.0) {
                            exit = Some((z[o as usize], o));
                        }
                    }
                }
            }
            let (ez, e) = exit.expect("exit");
            let away = steepest(e, &|o| body[o as usize] == id);
            let open = (ez - level).abs() < 0.01 && away != 0;
            if open {
                flow[e as usize] = away;
            }
            exits.insert(id, open.then_some(e));
        }
        T {
            world,
            z,
            body,
            surf,
            flow,
            exits,
            levels,
        }
    }

    impl T {
        fn candidates(&self) -> Vec<Candidate> {
            self.levels
                .iter()
                .map(|(&id, &level)| {
                    let mut bbox = [u64::MAX, u64::MAX, 0, 0];
                    let mut cells = 0;
                    for (c, &b) in self.body.iter().enumerate() {
                        if b == id {
                            let (x, y) = self.world.xy(c as u64);
                            bbox = [
                                bbox[0].min(x),
                                bbox[1].min(y),
                                bbox[2].max(x),
                                bbox[3].max(y),
                            ];
                            cells += 1;
                        }
                    }
                    Candidate {
                        id,
                        level,
                        exit: self.exits[&id],
                        bbox,
                        cells,
                        snowline: 0.0,
                    }
                })
                .collect()
        }
        fn run(&self, cands: &[Candidate]) -> (Vec<Built>, Outputs) {
            let w = self.world;
            let flow = Layer::memory(w, self.flow.clone());
            let lakes: BTreeMap<u32, (f32, Option<u64>)> = self
                .levels
                .iter()
                .map(|(&id, &lv)| (id, (lv, self.exits[&id])))
                .collect();
            let rivers = BTreeSet::new();
            let inp = Inputs {
                world: w,
                spacing_m: 1.0,
                flow: &flow,
                lakes: &lakes,
                rivers: &rivers,
            };
            let out = Outputs {
                ground: Layer::memory(w, self.z.clone()),
                body: Layer::memory(w, self.body.clone()),
                surface: Layer::memory(w, self.surf.clone()),
                mask: Layer::memory(w, vec![0; (w.w * w.h) as usize]),
                claimed: Layer::memory(w, vec![0; (w.w * w.h) as usize]),
            };
            (build(&inp, cands, &out).expect("build"), out)
        }
    }

    /// Cells a priority flood from the world's edge raises: the depressions.
    fn pits(world: World, z: &[f32]) -> Vec<u64> {
        let n = z.len();
        let mut seen = vec![false; n];
        let mut heap = BinaryHeap::new();
        for c in 0..n as u64 {
            let (x, y) = world.xy(c);
            if world.is_edge(x, y) {
                seen[c as usize] = true;
                heap.push(Reverse((Key(f64::from(z[c as usize])), c)));
            }
        }
        let mut out = Vec::new();
        while let Some(Reverse((Key(h), c))) = heap.pop() {
            let (x, y) = world.xy(c);
            for k in 0..8 {
                if let Some((nx, ny)) = world.neighbour(x, y, k) {
                    let o = world.index(nx, ny);
                    if !seen[o as usize] {
                        seen[o as usize] = true;
                        let v = f64::from(z[o as usize]);
                        if v < h {
                            out.push(o);
                        }
                        heap.push(Reverse((Key(v.max(h)), o)));
                    }
                }
            }
        }
        out
    }

    /// A valley falling west with two dammed lakes: 2 (upper) spills into 1, which spills
    /// west. Both are one glacier that holds no water, and the ground outside its
    /// catchment is unchanged.
    fn valley(x: u64, y: u64) -> f32 {
        let base = match x {
            0..=9 => 80.0 + 0.2 * x as f32,
            10 => 82.0,
            11..=30 => 75.0,
            31 => 90.0,
            32..=50 => 85.0,
            _ => 100.0 + 0.5 * (x - 50) as f32,
        };
        base + 0.3 * (y as f32 - 64.0).powi(2)
    }

    #[test]
    fn a_chain_of_lakes_fills_with_one_glacier_and_holds_no_water() {
        let t = make(
            160,
            128,
            valley,
            &[(1, 82.0, (20, 64)), (2, 90.0, (40, 64))],
        );
        assert!(t.exits[&1].is_some() && t.exits[&2].is_some());
        assert!(pits(t.world, &t.z).len() > 10, "the lakes are depressions");
        let (built, out) = t.run(&t.candidates());
        assert_eq!(built.len(), 1, "{built:?}");
        let b = &built[0];
        assert_eq!(b.skipped, None);
        assert_eq!(b.ids, [1, 2]);
        let (z, body, surf, mask) = (
            out.ground.to_vec().unwrap(),
            out.body.to_vec().unwrap(),
            out.surface.to_vec().unwrap(),
            out.mask.to_vec().unwrap(),
        );
        assert!(pits(t.world, &z).is_empty(), "no depression is left");
        for c in 0..z.len() {
            assert!(z[c] >= t.z[c], "never below the bed");
            if t.body[c] != 0 {
                assert_eq!((mask[c], body[c]), (255, 0), "lake cell {c} is ice");
                assert!(surf[c].is_nan());
            }
            if mask[c] == 0 {
                assert_eq!(z[c], t.z[c], "only ice changes the ground");
            }
        }
        // Rock far up the valley side and the outlet below the terminus are unchanged.
        let (tx, _) = (b.figures["terminus_cell"][0].as_u64().unwrap(), 0);
        assert!(tx < 10, "the terminus lies down the outlet");
        assert!((0..tx).all(|x| mask[t.world.index(x, 64) as usize] == 0));
        assert!(b.figures["max_thickness_m"].as_f64().unwrap() > 5.0);
    }

    /// A closed bowl (no outlet) spills its ice over the lowest col of its rim.
    #[test]
    fn a_closed_lake_spills_ice_over_its_col() {
        let bowl = |x: u64, y: u64| {
            let r = ((x as f32 - 100.0).powi(2) + (y as f32 - 100.0).powi(2)).sqrt();
            let rim = if r < 12.0 {
                0.05 * r * r
            } else if r < 20.0 {
                7.2 + 0.4 * (r - 12.0)
            } else {
                10.4 - 0.5 * (r - 20.0)
            };
            100.0 + rim + 0.02 * x as f32 * (r / 12.0).min(1.0)
        };
        let t = make(200, 200, bowl, &[(3, 104.0, (100, 100))]);
        assert_eq!(t.exits[&3], None, "closed");
        let (built, out) = t.run(&t.candidates());
        assert_eq!(built[0].skipped, None, "{built:?}");
        let z = out.ground.to_vec().unwrap();
        assert!(pits(t.world, &z).is_empty());
        let tx = built[0].figures["terminus_cell"][0].as_u64().unwrap();
        assert!(tx < 85, "spills west, over the lowest col");
    }

    /// Only ice lakes at or above the snowline are candidates.
    #[test]
    fn select_takes_ice_lakes_at_or_above_the_snowline() {
        let world = World { w: 10, h: 10 };
        let bodies = json!({"bodies": [
            {"id": 1, "kind": "lake", "level_m": 2100.0, "exit_cell": [3, 4]},
            {"id": 2, "kind": "lake", "level_m": 1900.0, "exit_cell": [5, 5]},
            {"id": 3, "kind": "lake", "level_m": 2500.0, "exit_cell": null},
            {"id": 4, "kind": "lake", "level_m": 2600.0, "exit_cell": [1, 1]}]});
        let ext = super::super::ice::Extent {
            bbox: [1, 1, 2, 2],
            cells: 4,
            sum: [6, 6],
        };
        let extents: BTreeMap<u32, _> = (1..=4).map(|id| (id, ext)).collect();
        let ice = BTreeSet::from([1, 2, 3]);
        let line = Snowline::flat(2000.0, 10.0);
        let c = select(&bodies, &ice, &extents, &line, world, |x, y| (x, 10.0 - y));
        let got: Vec<(u32, Option<u64>)> = c.iter().map(|c| (c.id, c.exit)).collect();
        assert_eq!(got, [(1, Some(43)), (3, None)]);
    }
}
