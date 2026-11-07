//! Hard checks (each a count that must be 0; spec "Checks", rulings R7, R11, R14, R16, R22,
//! R27, R29, R30, R31) and figures on a finished run, tile by tile with a 576-cell halo (every
//! straight run through a core cell, up to STRAIGHT_STEPS cells, must fit).

use super::bodies::main_up;
use super::geom::{OFFSETS, Rect, UnionFind, step_len};
use super::layers::Layer;
use super::pipeline::Setup;
use super::{BODY_BUDGET, MAIN_RIVER_KM2};
use crate::acquisition::Result;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const HALO: u64 = 576;
/// Parallel rivers (R29): another flow line from this size, this close, this parallel, this long.
/// The size is the river threshold (ruling after the 8192² run): a doubled channel carries
/// river-sized flow, while hillslope lines of 0.002–0.03 km² run beside rivers naturally.
const PARALLEL_KM2: f32 = 0.05;
const PARALLEL_M: f64 = 10.0;
const PARALLEL_COS: f64 = 0.7;
const PARALLEL_RUN_M: f64 = 100.0;
/// Cells beside a river cell: at most this far (cells) along the river from it, so a diagonal
/// river sees every cell across, at odd and even offsets alike.
const PARALLEL_ALONG: f64 = 0.75;
/// A doubled channel's run bridges unflagged gaps up to this long.
const PARALLEL_GAP_M: f64 = 5.0;
/// Straight runs (R30) at any angle: a thalweg path whose every cell lies within this distance
/// of the chord from its first to its last cell. Judgment value: a D8 line at any angle stays
/// within half a cell of its chord, so 1 m catches tool-made lines; a ±1.5 m wobble does not
/// count (at 1.5 m the straight share of natural reaches in straight raw valleys rose from
/// 1-4 % to 13-26 % on the probes).
const STRAIGHT_DEV_M: f64 = 1.0;
/// Runs longer than this fail unless the raw thalweg is straight there too.
const STRAIGHT_RUN_M: f64 = 150.0;
/// A run is followed at most this many cells (longer ones are reported at this length).
const STRAIGHT_STEPS: usize = 400;
const GORGE_ACROSS: i64 = 3;
const GORGE_SHARE: f64 = 0.9;
/// Label agreement (R27).
const AGREEMENT: f64 = 0.95;
/// |Δ| histogram: 1 cm bins to 10 m, the last bin holds the rest.
pub(crate) const BINS: usize = 1001;
const RISE_M: f32 = 1e-4;
pub(crate) const SILL_M: f32 = 0.01;
const DAM_STEP_M: f32 = 1.5;
const DAM_RIM_M: f32 = 0.5;
/// Exit falls are read this far below each exit (figures; the 50 m one is also `dam_steps`'s
/// reach).
const FALL_M: [f64; 3] = [5.0, 10.0, 50.0];
const ACROSS_M: f64 = 30.0;
const THALWEG_M: f32 = 1.0;
/// One thalweg: the scan across stops at a ridge this far above the river's surface, and of
/// two rivers where one's routing reaches the other's body within TRIBUTARY_M, one is the
/// other's tributary.
const RIDGE_M: f32 = 0.5;
const TRIBUTARY_M: f64 = 150.0;
const STRAIGHT_M: f64 = 60.0;
const STRAIGHT_SHARE: f64 = 0.05;
const PLANE_RMS_M: f64 = 0.02;
const PLANE_DEG: f64 = 5.0;
/// A cell changed by more than this counts as changed (walls, facets).
pub(crate) const CHANGED_M: f32 = 0.01;
/// Bank water standing this far above the dry cell it drains into (a figure, I5).
const BANK_STEP_M: f32 = 0.5;

pub(crate) struct CheckLayers {
    pub raw: Layer<f32>,
    pub ground: Layer<f32>,
    pub surface: Layer<f32>,
    pub body: Layer<u32>,
    pub level: Layer<f32>,
    pub node: Layer<u32>,
    pub flow: Layer<u8>,
    pub discharge: Layer<f32>,
    /// The written rivers' flow layer (m2), for the label agreement with m3's `flow`.
    pub flow_written: Layer<u8>,
    /// The m2 node layer and the ground channels were cut into, for `bed_rises`.
    pub node_written: Layer<u32>,
    /// The m2 level layer, for `m2_pond_cells_lost`.
    pub level_written: Layer<f32>,
    pub breached: Layer<f32>,
}

/// Lakes by id: level, full, outlet [inside, outside], cells needed (closed).
pub(crate) type LakeInfo = BTreeMap<u32, (f32, bool, [u64; 2], u64)>;
/// Lake and sea levels by id as the export reads `bodies.json` (`lake_level_json`).
pub(crate) type Levels = BTreeMap<u32, f32>;

/// What the export reads from `bodies.json`: lake and sea levels and every water body id (the
/// export refuses a body id it does not list).
pub(crate) struct Listed {
    pub levels: Levels,
    pub ids: BTreeSet<u32>,
}

/// A full node's exit (a lake, an R26 pool or a pond), walked for the dam figures; a listed
/// lake's also for the hard exit checks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Exit {
    pub key: u64,
    pub level: f32,
    /// [cell inside, cell outside] of the spill edge.
    pub outlet: [u64; 2],
    /// The lake's body id, when the node is a listed lake.
    pub lake: Option<u32>,
}

/// The fall below an exit: lv − D where the path has gone 5, 10 and 50 m (or where it ends).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fall {
    pub key: u64,
    pub lake: Option<u32>,
    pub cell: [u64; 2],
    pub m: [f32; 3],
}

/// A dry cell beside an exit above its level: (node key, lake, cell, after m, before m).
type Rim = (u64, Option<u32>, [u64; 2], f32, f32);

/// A review spot (east, north in metres) and its cell, when inside the grid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spot {
    pub en: [f64; 2],
    pub cell: Option<[u64; 2]>,
}

/// An exit-walk drop over DAM_STEP_M near a spot: (spot index, node key, lake, cell, drop m,
/// raw drop m).
type SpotDrop = (usize, u64, Option<u32>, [u64; 2], f32, f32);
/// Counted like checks but reported as figures `<name>_count` (owner ruling 2026-11-06).
const FIGURE_ONLY: [&str; 3] = ["parallel_rivers", "straight_runs", "straight_share"];
/// Spot drops are recorded this far from their spot.
const SPOT_M: f64 = 100.0;
/// `unlabelled_main_river_cells` also counts runs of this many unlabelled river cells.
const UNLABELLED_RUN_CELLS: usize = 10;

/// A cluster of overlapping straight thalweg runs (R30), by its longest member.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    pub length_m: f64,
    pub from: [u64; 2],
    pub to: [u64; 2],
    /// Share of its cells in ponds (flat routing) and on the raw thalweg across.
    pub pond: f64,
    pub raw: f64,
    /// Overlapping maximal runs merged into this one.
    pub members: usize,
}

#[derive(Default)]
pub(crate) struct Tile {
    pub fails: BTreeMap<&'static str, u64>,
    pub examples: BTreeMap<&'static str, Vec<[u64; 2]>>,
    pub hist: Vec<u64>,
    /// |Δ| over changed cells only.
    pub hist_changed: Vec<u64>,
    pub max_change: f32,
    pub max_q: f32,
    pub planar: [u64; 2],
    pub straight_m: f64,
    pub river_m: f64,
    pub beside_m: f64,
    pub pond_cells: u64,
    pub falls: Vec<Fall>,
    /// Lowest saddle out of each lake seen in this tile (lake id → min of max(level, D)).
    pub saddle: BTreeMap<u32, f32>,
    /// Straight-run clusters over 60 m whose longest member starts in this tile (a figure,
    /// approximate across tiles: a window can split a cluster).
    pub runs: Vec<Run>,
    /// Of the `straight_runs` cells, those in ponds (reported apart, still failing).
    pub pond_run_cells: u64,
    /// Raw-independent dam figures (R31): (node key, lake, cell, drop m).
    pub drops_any: Vec<(u64, Option<u32>, [u64; 2], f32)>,
    /// Dry cells beside an exit above its level: (node key, lake, cell, after m, before m).
    pub rims_any: Vec<Rim>,
    /// Exit-walk drops near the review spots (`dam_drops_at_spots`).
    pub spot_drops: Vec<SpotDrop>,
    /// Dry m3 thalweg cells in no node (`unlabelled_main_river_cells`): those with Q ≥ 1 km²,
    /// runs of UNLABELLED_RUN_CELLS or more at the river threshold and their cells, and the
    /// largest-Q cells (Q, cell).
    pub unlabelled: (u64, u64, u64, Vec<(f32, [u64; 2])>),
    /// Label agreement (R27): written thalweg cells, of them matched within one cell, m3
    /// thalweg cells, of them matched.
    pub agree: [u64; 4],
    /// Rivers with Q ≥ 1 km² ending in a closed node (M2).
    pub closed_ends: u64,
    /// `thalweg_beside` cells excused as tributaries: [p's channel joins j's body, j joins p's].
    pub tributary_excused: [u64; 2],
    /// Bank water over 0.5 m above the dry cell it drains into: count, highest, examples.
    pub bank_steps: (u64, f32, Vec<[u64; 2]>),
    /// Cells per class, and (class, lake) pairs seen.
    pub class_cells: BTreeMap<u8, u64>,
    pub class_lakes: BTreeSet<(u8, u32)>,
    /// Largest |m3 level − label level| over closed lakes' cells: (m, lake id).
    pub lake_m3_gap: Option<(f32, u32)>,
    /// m2 node cells in no m3 node, and the largest m2 level − m3 level (ground where dry)
    /// over m2 node cells: (m, m2 node, cell [x, y]).
    pub pond_lost: (u64, Option<PondDrop>),
}

type PondDrop = (f32, u32, [u64; 2]);

/// The larger drop; ties keep the smaller node, then the smaller cell (row first).
fn deeper_drop(a: Option<PondDrop>, b: Option<PondDrop>) -> Option<PondDrop> {
    let key = |d: &PondDrop| (d.1, d.2[1], d.2[0]);
    match (a, b) {
        (Some(x), Some(y)) => Some(if y.0 > x.0 || (y.0 == x.0 && key(&y) < key(&x)) {
            y
        } else {
            x
        }),
        _ => a.or(b),
    }
}

/// The larger gap; ties keep the smaller lake id.
fn wider_gap(a: Option<(f32, u32)>, b: Option<(f32, u32)>) -> Option<(f32, u32)> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if y.0 > x.0 || (y.0 == x.0 && y.1 < x.1) {
            y
        } else {
            x
        }),
        _ => a.or(b),
    }
}

impl Tile {
    fn fail(&mut self, name: &'static str, x: u64, y: u64) {
        *self.fails.entry(name).or_insert(0) += 1;
        let e = self.examples.entry(name).or_default();
        if e.len() < 5 {
            e.push([x, y]);
        }
    }
}

/// Planar 5 x 5 window: least-squares plane residual under 2 cm and slope over 5°.
pub(crate) fn planar(z: &[f32], spacing_m: f64) -> bool {
    let mean = z.iter().map(|&v| f64::from(v)).sum::<f64>() / 25.0;
    let (mut sx, mut sy) = (0.0, 0.0);
    for (i, &v) in z.iter().enumerate() {
        let (x, y) = ((i % 5) as f64 - 2.0, (i / 5) as f64 - 2.0);
        sx += x * f64::from(v);
        sy += y * f64::from(v);
    }
    let (a, b) = (sx / 50.0, sy / 50.0);
    let rms = (z
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let (x, y) = ((i % 5) as f64 - 2.0, (i / 5) as f64 - 2.0);
            (f64::from(v) - mean - a * x - b * y).powi(2)
        })
        .sum::<f64>()
        / 25.0)
        .sqrt();
    rms < PLANE_RMS_M && (a.hypot(b) / spacing_m).atan().to_degrees() > PLANE_DEG
}

/// Horn slope in degrees at (x, y) of a `w`-wide block.
pub(crate) fn horn_deg(z: &[f32], w: usize, x: usize, y: usize, spacing_m: f64) -> f64 {
    let at =
        |dx: i64, dy: i64| f64::from(z[(y as i64 + dy) as usize * w + (x as i64 + dx) as usize]);
    let gx = (at(1, -1) + 2.0 * at(1, 0) + at(1, 1) - at(-1, -1) - 2.0 * at(-1, 0) - at(-1, 1))
        / (8.0 * spacing_m);
    let gy = (at(-1, 1) + 2.0 * at(0, 1) + at(1, 1) - at(-1, -1) - 2.0 * at(0, -1) - at(1, -1))
        / (8.0 * spacing_m);
    gx.hypot(gy).atan().to_degrees()
}

/// A changed cell's slope (Horn, degrees) fails `walls` against its raw slope and the level's
/// angle (B8; shared with breach, channels and erosion).
pub(crate) fn is_wall(slope: f64, raw_slope: f64, wall_deg: f64) -> bool {
    slope > wall_deg.max(raw_slope) + 0.01
}

pub(crate) fn p99_m(hist: &[u64]) -> f64 {
    let total: u64 = hist.iter().sum();
    let want = (total as f64 * 0.99).ceil() as u64;
    let mut seen = 0;
    for (b, &n) in hist.iter().enumerate() {
        seen += n;
        if seen >= want {
            return b as f64 / 100.0;
        }
    }
    (BINS - 1) as f64 / 100.0
}

/// Angle difference wrapped into (−π, π].
fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let r = a.rem_euclid(t);
    if r > std::f64::consts::PI { r - t } else { r }
}

/// A maximal straight run: start, length m, end, cells (window indices).
type Maximal = (usize, f64, usize, Vec<usize>);

pub(crate) fn check_tile(
    s: &Setup,
    l: &CheckLayers,
    core: Rect,
    lakes: &LakeInfo,
    listed: &Listed,
    exits: &[Exit],
    spots: &[Spot],
) -> Result<Tile> {
    let world = s.world;
    let win = core.grow(HALO, world);
    let (raw, z, surf, body, level, node, flow, q) = (
        l.raw.read(win)?,
        l.ground.read(win)?,
        l.surface.read(win)?,
        l.body.read(win)?,
        l.level.read(win)?,
        l.node.read(win)?,
        l.flow.read(win)?,
        l.discharge.read(win)?,
    );
    let fw = l.flow_written.read(win)?;
    let (n2, bz) = (l.node_written.read(win)?, l.breached.read(win)?);
    let l2 = l.level_written.read(win)?;
    let classes = match &s.classes {
        Some(c) => c.read(core)?,
        None => vec![0; core.cells()],
    };
    let at = |x: u64, y: u64| win.local(x, y);
    let at_i = |x: i64, y: i64| (x >= 0 && y >= 0).then(|| at(x as u64, y as u64)).flatten();
    let gi = |j: usize| {
        let (x, y) = win.world_xy(j);
        world.index(x, y)
    };
    let around = |j: usize| {
        let (x, y) = win.world_xy(j);
        (0..8).filter_map(move |k| {
            world
                .neighbour(x, y, k)
                .and_then(|(nx, ny)| win.local(nx, ny))
        })
    };
    let down = |j: usize| -> Option<usize> {
        let d = flow[j] & 15;
        if d == 0 {
            return None;
        }
        let (x, y) = win.world_xy(j);
        world
            .neighbour(x, y, d as usize - 1)
            .and_then(|(nx, ny)| at(nx, ny))
    };
    let lake_cell = |j: usize| surf[j].is_finite() && lakes.contains_key(&body[j]);
    // River thalwegs of the final routing; labelled lakes are no river length.
    let thal = |j: usize| flow[j] & 16 != 0 && !lake_cell(j);
    // What drains: labelled water at its surface (wetted river-crossed ponds included), ponds
    // at their level, else ground.
    let dsurf = |j: usize| {
        if surf[j].is_finite() {
            surf[j]
        } else if level[j].is_finite() {
            level[j]
        } else {
            z[j]
        }
    };
    let shown = |j: usize| if surf[j].is_finite() { surf[j] } else { z[j] };
    let receiver = |x: u64, y: u64, j: usize| -> Option<(u64, u64)> {
        let d = flow[j] & 15;
        (d != 0)
            .then(|| world.neighbour(x, y, d as usize - 1))
            .flatten()
    };
    let main_river = |j: usize| thal(j) && f64::from(q[j]) >= MAIN_RIVER_KM2;
    // Unit flow direction over 5 steps downstream of window cell j.
    let direction = |j: usize| -> Option<(f64, f64)> {
        let (x, y) = win.world_xy(j);
        let mut c = j;
        for _ in 0..5 {
            let Some(n) = down(c) else { break };
            c = n;
        }
        let (cx, cy) = win.world_xy(c);
        let (vx, vy) = (cx as f64 - x as f64, cy as f64 - y as f64);
        let len = vx.hypot(vy);
        (len > 0.0).then(|| (vx / len, vy / len))
    };

    // Another flow line beside a main river (R29), seen from river cell j with unit direction
    // v: every cell within 10 m across and at most 0.75 cell along.
    let reach_par = (PARALLEL_M / s.spacing_m).ceil() as i64;
    let parallel_at = |j: usize, v: (f64, f64)| -> bool {
        let (x, y) = win.world_xy(j);
        for oy in -reach_par..=reach_par {
            for ox in -reach_par..=reach_par {
                let (fx, fy) = (ox as f64, oy as f64);
                let (along, across) = (fx * v.0 + fy * v.1, fx * v.1 - fy * v.0);
                if (ox, oy) == (0, 0)
                    || along.abs() > PARALLEL_ALONG
                    || across.abs() * s.spacing_m > PARALLEL_M
                {
                    continue;
                }
                let Some(p) = at_i(x as i64 + ox, y as i64 + oy) else {
                    continue;
                };
                let d = flow[p] & 15;
                if q[p] < PARALLEL_KM2 || d == 0 || (surf[p].is_finite() && body[p] == body[j]) {
                    continue;
                }
                let (dy, dx) = OFFSETS[d as usize - 1];
                let len = ((dx * dx + dy * dy) as f64).sqrt();
                if (dx as f64 * v.0 + dy as f64 * v.1) / len < PARALLEL_COS {
                    continue;
                }
                // It joins the river within 100 m downstream: a tributary, not a second channel.
                let (mut c, mut walked, mut joins) = (p, 0.0, false);
                while walked <= PARALLEL_RUN_M {
                    if surf[c].is_finite() && body[c] == body[j] {
                        joins = true;
                        break;
                    }
                    let dc = flow[c] & 15;
                    let Some(n) = down(c) else { break };
                    walked += step_len(dc) * s.spacing_m;
                    c = n;
                }
                if !joins {
                    return true;
                }
            }
        }
        false
    };
    let memo: RefCell<HashMap<usize, bool>> = RefCell::new(HashMap::new());
    let flagged = |j: usize| {
        if let Some(&f) = memo.borrow().get(&j) {
            return f;
        }
        let f = main_river(j) && direction(j).is_some_and(|v| parallel_at(j, v));
        memo.borrow_mut().insert(j, f);
        f
    };
    let up = |j: usize| {
        let (x, y) = win.world_xy(j);
        main_up(world, win, &flow, &q, x, y).and_then(|u| {
            let (ux, uy) = world.xy(u);
            at(ux, uy)
        })
    };
    // A flagged cell within the gap bridge upstream: j continues that run.
    let flagged_above = |j: usize| {
        let (mut c, mut walked) = (j, 0.0);
        while let Some(u) = up(c) {
            walked += step_len(flow[u] & 15) * s.spacing_m;
            if walked > PARALLEL_GAP_M + 1e-9 {
                return false;
            }
            if flagged(u) {
                return true;
            }
            c = u;
        }
        false
    };

    // The lowest D over cells of body `id` within `r` cells of window cell j (D(j) included):
    // downstream own cells within reach count, so a steep reach is judged at its own grade.
    let reach_across = (ACROSS_M / s.spacing_m).round() as i64;
    let own_min = |j: usize, r: f64, id: u32| -> f32 {
        let (x, y) = win.world_xy(j);
        let reach = r.ceil() as i64;
        let mut low = dsurf(j);
        for oy in -reach..=reach {
            for ox in -reach..=reach {
                if ((ox * ox + oy * oy) as f64).sqrt() > r {
                    continue;
                }
                if let Some(c) = at_i(x as i64 + ox, y as i64 + oy)
                    && body[c] == id
                {
                    low = low.min(dsurf(c));
                }
            }
        }
        low
    };

    // Straight runs at any angle (R30): from each thalweg cell, the longest downstream thalweg
    // path whose cells all lie within `tol` of its chord: (length m, end).
    let tol = (STRAIGHT_DEV_M / s.spacing_m).max(0.75);
    let straight_from = |j: usize| -> (f64, usize) {
        let (x0, y0) = win.world_xy(j);
        let (mut c, mut walked, mut best) = (j, 0.0, (0.0, j));
        let (mut lo, mut hi, mut reference) = (f64::NEG_INFINITY, f64::INFINITY, None);
        for _ in 0..STRAIGHT_STEPS {
            let Some(n) = down(c).filter(|&n| thal(n)) else {
                break;
            };
            walked += step_len(flow[c] & 15);
            c = n;
            let (x, y) = win.world_xy(c);
            let (vx, vy) = (x as f64 - x0 as f64, y as f64 - y0 as f64);
            let r = vx.hypot(vy);
            let a = vy.atan2(vx);
            let rel = wrap(a - *reference.get_or_insert(a));
            if rel >= lo && rel <= hi {
                best = (walked * s.spacing_m, c);
            }
            // Every later chord must pass within `tol` of this cell.
            if r > tol {
                let half = (tol / r).asin();
                (lo, hi) = (lo.max(rel - half), hi.min(rel + half));
                if lo > hi {
                    break;
                }
            }
        }
        best
    };
    let mut runs_at: HashMap<usize, (f64, usize)> = HashMap::new();
    for j in 0..win.cells() {
        if thal(j) {
            runs_at.insert(j, straight_from(j));
        }
    }
    // Only maximal runs count: one inside a longer run (which starts upstream on the same
    // path and ends no earlier) is part of it. Longest first, so containers come first; each
    // maximal run's cells keep the distance left to its end.
    let mut starts: Vec<(f64, usize)> = runs_at
        .iter()
        .filter(|&(_, &(len, _))| len > STRAIGHT_M)
        .map(|(&j, &(len, _))| (len, j))
        .collect();
    starts.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut left: HashMap<usize, f64> = HashMap::new();
    let mut maximal: Vec<Maximal> = Vec::new();
    for (len, j) in starts {
        if left.get(&j).is_some_and(|&rem| rem + 1e-6 >= len) {
            continue;
        }
        let end = runs_at[&j].1;
        let mut cells = vec![j];
        while *cells.last().expect("start") != end {
            let next = down(*cells.last().expect("start")).expect("run path");
            cells.push(next);
        }
        let mut rem = len;
        for (i, &c) in cells.iter().enumerate() {
            let e = left.entry(c).or_insert(0.0);
            *e = e.max(rem);
            if i + 1 < cells.len() {
                rem -= step_len(flow[c] & 15) * s.spacing_m;
            }
        }
        maximal.push((j, len, end, cells));
    }
    // Overlapping maximal runs (sharing a cell) are one straight reach: one cluster.
    let mut uf = UnionFind::new(maximal.len());
    let mut owner: HashMap<usize, u32> = HashMap::new();
    for (r, m) in maximal.iter().enumerate() {
        for &c in &m.3 {
            match owner.get(&c) {
                Some(&o) => {
                    uf.union(o, r as u32);
                }
                None => {
                    owner.insert(c, r as u32);
                }
            }
        }
    }
    let mut clusters: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for r in 0..maximal.len() {
        clusters.entry(uf.find(r as u32)).or_default().push(r);
    }
    // Share of a run's cells where the lowest raw cell across (±3) is a run cell; an exact
    // tie with a cell off the run does not count.
    let raw_share = |m: &Maximal| -> f64 {
        let (x, y) = win.world_xy(m.0);
        let (ex, ey) = win.world_xy(m.2);
        let on: HashSet<usize> = m.3.iter().copied().collect();
        let (vx, vy) = (ex as f64 - x as f64, ey as f64 - y as f64);
        let vl = vx.hypot(vy).max(1e-9);
        let (nx, ny) = (-vy / vl, vx / vl);
        let count =
            m.3.iter()
                .filter(|&&c| {
                    let (cx, cy) = win.world_xy(c);
                    let mut low = (raw[c], true);
                    for i in (-GORGE_ACROSS..=GORGE_ACROSS).filter(|&i| i != 0) {
                        let (px, py) = (
                            (cx as f64 + nx * i as f64).round() as i64,
                            (cy as f64 + ny * i as f64).round() as i64,
                        );
                        let Some(p) = at_i(px, py) else {
                            continue;
                        };
                        if raw[p] < low.0 {
                            low = (raw[p], on.contains(&p));
                        } else if raw[p] == low.0 {
                            low.1 &= on.contains(&p);
                        }
                    }
                    low.1
                })
                .count();
        count as f64 / m.3.len() as f64
    };
    let pond_share =
        |m: &Maximal| m.3.iter().filter(|&&c| node[c] >= 2).count() as f64 / m.3.len() as f64;
    let mut t = Tile {
        hist: vec![0; BINS],
        hist_changed: vec![0; BINS],
        ..Default::default()
    };
    for members in clusters.values() {
        // The longest member stands for the cluster (ties: the smaller start index).
        let rep = *members
            .iter()
            .max_by(|&&a, &&b| {
                maximal[a]
                    .1
                    .total_cmp(&maximal[b].1)
                    .then(gi(maximal[b].0).cmp(&gi(maximal[a].0)))
            })
            .expect("a cluster has members");
        let m = &maximal[rep];
        let (x, y) = win.world_xy(m.0);
        if !core.contains(x, y) {
            continue;
        }
        let (ex, ey) = win.world_xy(m.2);
        t.runs.push(Run {
            length_m: m.1,
            from: [x, y],
            to: [ex, ey],
            pond: pond_share(m),
            raw: raw_share(m),
            members: members.len(),
        });
    }
    // The hard count (scale diagnosis 4): core thalweg cells on any failing run (over 150 m,
    // raw thalweg share under 0.9) from any start. Every such run lies within STRAIGHT_STEPS
    // cells of each of its cells, inside the halo, so the count is exact over tiles.
    let mut covered: HashSet<usize> = HashSet::new();
    for (&j, &(len, end)) in &runs_at {
        if len <= STRAIGHT_RUN_M {
            continue;
        }
        let mut cells = vec![j];
        while *cells.last().expect("start") != end {
            cells.push(down(*cells.last().expect("start")).expect("run path"));
        }
        let run = (j, len, end, cells);
        if raw_share(&run) < GORGE_SHARE {
            covered.extend(run.3);
        }
    }
    let mut covered: Vec<usize> = covered
        .into_iter()
        .filter(|&c| {
            let (x, y) = win.world_xy(c);
            core.contains(x, y)
        })
        .collect();
    covered.sort_unstable();
    for c in covered {
        let (x, y) = win.world_xy(c);
        t.fail("straight_runs", x, y);
        t.pond_run_cells += u64::from(node[c] >= 2);
    }

    let thr = s.river_km2();
    for (k, &class) in classes.iter().enumerate() {
        let (x, y) = core.world_xy(k);
        let j = at(x, y).expect("core in window");
        *t.class_cells.entry(class).or_insert(0) += 1;
        t.max_q = t.max_q.max(q[j]);
        // Change budget.
        let delta = (z[j] - raw[j]).abs();
        let bin = ((delta * 100.0).round() as usize).min(BINS - 1);
        t.hist[bin] += 1;
        if delta > CHANGED_M {
            t.hist_changed[bin] += 1;
        }
        t.max_change = t.max_change.max(delta);
        if delta > s.params.change_max_m() {
            t.fail("change_max", x, y);
        }
        // The export's contract (export/water.rs `level0`): a lake or sea cell stands at its
        // bodies.json level (exactly, so any height encoding agrees), and every labelled cell
        // is a body bodies.json lists with a surface strictly above the ground.
        if body[j] == 0 {
            if surf[j].is_finite() {
                t.fail("surface_without_body", x, y);
            }
        } else {
            if !listed.ids.contains(&body[j]) {
                t.fail("wet_unknown_body", x, y);
            }
            let sj = match listed.levels.get(&body[j]) {
                Some(&lv) if surf[j].is_finite() => {
                    if surf[j] != lv {
                        t.fail("lake_level_json", x, y);
                    }
                    lv
                }
                _ => surf[j],
            };
            if !sj.is_finite() {
                t.fail("wet_without_surface", x, y);
            } else if sj <= z[j] {
                t.fail("wet_not_above_ground", x, y);
            }
        }
        // Unlabelled rivers (a figure): dry m3 thalweg cells in no node; runs counted from
        // their upstream end.
        let unlabelled =
            |c: usize| flow[c] & 16 != 0 && !surf[c].is_finite() && node[c] == 0 && q[c] >= thr;
        if unlabelled(j) {
            if f64::from(q[j]) >= MAIN_RIVER_KM2 {
                t.unlabelled.0 += 1;
                t.unlabelled.3.push((q[j], [x, y]));
            }
            if !around(j).any(|p| unlabelled(p) && down(p) == Some(j)) {
                let (mut c, mut n) = (j, 1usize);
                while let Some(d) = down(c).filter(|&d| unlabelled(d)) {
                    (c, n) = (d, n + 1);
                    if n > win.cells() {
                        break;
                    }
                }
                if n >= UNLABELLED_RUN_CELLS {
                    t.unlabelled.1 += 1;
                    t.unlabelled.2 += n as u64;
                }
            }
        }
        if node[j] >= 2 && !lake_cell(j) {
            t.pond_cells += 1;
        }
        // A figure (fix round 2): m2 pond water the carved ground's routing no longer holds.
        if n2[j] >= 2 {
            t.pond_lost.0 += u64::from(node[j] == 0);
            let l3 = if level[j].is_finite() { level[j] } else { z[j] };
            let drop = Some((l2[j] - l3, n2[j], [x, y]));
            t.pond_lost.1 = deeper_drop(t.pond_lost.1, drop);
        }
        // Drainage.
        if node[j] != 1 {
            match receiver(x, y, j) {
                Some((rx, ry)) => {
                    let r = at(rx, ry).expect("receiver in window");
                    if dsurf(r) > dsurf(j) + RISE_M {
                        t.fail("drainage_rises", x, y);
                    }
                    if surf[j].is_finite() && surf[r].is_nan() && surf[j] - z[r] > BANK_STEP_M {
                        let b = &mut t.bank_steps;
                        b.0 += 1;
                        b.1 = b.1.max(surf[j] - z[r]);
                        if b.2.len() < 5 {
                            b.2.push([x, y]);
                        }
                    }
                }
                None => {
                    let dry = node[j] == 0 && surf[j].is_nan();
                    if dry && !world.is_edge(x, y) && flow[j] & 32 == 0 {
                        t.fail("drainage_stuck", x, y);
                    }
                }
            }
        }
        // Bed grade: on a dry m2 thalweg link where the ground (before channels) falls, the
        // carved bed does not rise (beds under standing water may).
        let d2 = fw[j] & 15;
        if fw[j] & 16 != 0
            && n2[j] == 0
            && d2 != 0
            && let Some(r) = world
                .neighbour(x, y, d2 as usize - 1)
                .and_then(|(rx, ry)| at(rx, ry))
            && n2[r] == 0
            && bz[r] <= bz[j]
            && z[r] > z[j] + RISE_M
        {
            t.fail("bed_rises", x, y);
        }
        // Lakes are flat, stand at their lowest saddle and do not leak (C1). A lake cell
        // whose ground reaches its level holds no water: it is shore on both sides, never
        // a leak's source, and a neighbour at the level (D = level).
        if let Some(&(lv, full, outlet, _)) = lake_cell(j).then(|| &lakes[&body[j]]) {
            t.class_lakes.insert((class, body[j]));
            if surf[j] != lv {
                t.fail("lake_not_flat", x, y);
            }
            // A figure (gate 1 ruling): the lake stays at its label, m3 may balance higher.
            if !full && level[j].is_finite() {
                t.lake_m3_gap = wider_gap(t.lake_m3_gap, Some(((level[j] - lv).abs(), body[j])));
            }
            if z[j] < lv {
                for n in around(j) {
                    let same = surf[n].is_finite() && body[n] == body[j];
                    if same && z[n] < lv {
                        continue;
                    }
                    let dn = if same { lv } else { dsurf(n) };
                    let e = t.saddle.entry(body[j]).or_insert(f32::INFINITY);
                    *e = e.min(lv.max(dn));
                    if dn < lv - SILL_M && !(full && gi(n) == outlet[1]) {
                        let (nx, ny) = win.world_xy(n);
                        t.fail("lake_leaks", nx, ny);
                    }
                }
            }
        }
        // Label agreement (R27): written thalwegs (m2) vs the final routing's (m3), within one cell.
        let thal_m2 = fw[j] & 16 != 0 && surf[j].is_finite();
        let thal_m3 = flow[j] & 16 != 0;
        if thal_m2 {
            t.agree[0] += 1;
            t.agree[1] += u64::from(thal_m3 || around(j).any(|n| flow[n] & 16 != 0));
        }
        if thal_m3 {
            t.agree[2] += 1;
            t.agree[3] +=
                u64::from(thal_m2 || around(j).any(|n| fw[n] & 16 != 0 && surf[n].is_finite()));
        }
        // Walls and facets on changed cells. Their stencils (Horn 3 × 3, facet 5 × 5) need two
        // cells all round, so the outermost two rows and columns of the world are not checked
        // (inside the world the halo always holds them).
        let inner = x >= win.x0 + 2
            && y >= win.y0 + 2
            && x + 2 < win.x0 + win.w as u64
            && y + 2 < win.y0 + win.h as u64;
        if delta > CHANGED_M && inner {
            let (lx, ly) = ((x - win.x0) as usize, (y - win.y0) as usize);
            let (fs, rs) = (
                horn_deg(&z, win.w, lx, ly, s.spacing_m),
                horn_deg(&raw, win.w, lx, ly, s.spacing_m),
            );
            if is_wall(fs, rs, s.params.wall_deg) {
                t.fail("walls", x, y);
            }
            let block = |v: &[f32]| -> Vec<f32> {
                (0..25)
                    .map(|i| v[(ly + i / 5 - 2) * win.w + lx + i % 5 - 2])
                    .collect()
            };
            t.planar[0] += u64::from(planar(&block(&raw), s.spacing_m));
            t.planar[1] += u64::from(planar(&block(&z), s.spacing_m));
        }
        // Rivers: straightness, one thalweg, rivers beside rivers.
        if thal(j) && q[j] >= thr {
            let d = flow[j] & 15;
            let step = step_len(d) * s.spacing_m;
            t.river_m += step;
            // On a straight run over 60 m (its last cell's step leaves the run).
            if left.get(&j).is_some_and(|&rem| rem > 1e-6) {
                t.straight_m += step;
            }
            // A big river ending in a closed node (M2).
            if f64::from(q[j]) >= MAIN_RIVER_KM2
                && let Some(r) = down(j)
                && node[r] >= 2
                && flow[r] & 64 != 0
            {
                t.closed_ends += 1;
            }
            // Parallel rivers (R29): flagged main-river cells over more than 100 m, gaps up to
            // 5 m bridged, counted from the run's upstream end.
            if flagged(j) && !flagged_above(j) {
                let (mut c, mut walked, mut last, mut gap) = (j, 0.0, 0.0, 0.0);
                while walked <= PARALLEL_RUN_M + PARALLEL_GAP_M {
                    let dc = flow[c] & 15;
                    let Some(n) = down(c).filter(|&n| main_river(n)) else {
                        break;
                    };
                    let step = step_len(dc) * s.spacing_m;
                    walked += step;
                    c = n;
                    if flagged(c) {
                        (last, gap) = (walked, 0.0);
                    } else {
                        gap += step;
                        if gap > PARALLEL_GAP_M + 1e-9 {
                            break;
                        }
                    }
                }
                if last > PARALLEL_RUN_M {
                    t.fail("parallel_rivers", x, y);
                }
            }
            // One thalweg (I3, spec): across the flow, out to the first ridge on each side (more
            // than 0.5 m above D here), the lowest cell where water stands (raw, a pond's
            // level, shown water: a filled pit or a lake is no second thalweg) is a cell of
            // this river's body (wetted river-crossed ponds included), lies on a tributary
            // beside its trunk (one channel drains into the other's water within 150 m; a dry
            // hollow draining back is no channel), lies no more than 1 m below the lowest D of
            // this body within its distance + 1 cell, or carries no channel (see the ruling
            // below). Too low a river is no failure: channels cut 2 m at most, and `walls` /
            // `change_max` cover over-cutting.
            if f64::from(q[j]) >= MAIN_RIVER_KM2
                && let Some((vx, vy)) = direction(j)
            {
                let (nx, ny) = (-vy, vx);
                let dj = dsurf(j);
                let stands = |p: usize| raw[p].max(level[p]).max(surf[p]);
                let mut low = (stands(j), 0i64, j);
                for side in [-1i64, 1] {
                    for i in 1..=reach_across {
                        let Some(p) = at_i(
                            (x as f64 + nx * (side * i) as f64).round() as i64,
                            (y as f64 + ny * (side * i) as f64).round() as i64,
                        ) else {
                            break;
                        };
                        let u = stands(p);
                        if u > dj + RIDGE_M {
                            break;
                        }
                        if u < low.0 || (u == low.0 && i < low.1) {
                            low = (u, i, p);
                        }
                    }
                }
                let (zl, _, p) = low;
                // Whether the path from `from` reaches water of body `id` within 150 m.
                let joins = |from: usize, id: u32| {
                    let (mut c, mut walked) = (from, 0.0);
                    while walked <= TRIBUTARY_M {
                        if surf[c].is_finite() && body[c] == id {
                            return true;
                        }
                        let dc = flow[c] & 15;
                        let Some(n) = down(c) else { return false };
                        walked += step_len(dc) * s.spacing_m;
                        c = n;
                    }
                    false
                };
                // The river j belongs to: its body, or for an unlabelled m3 thalweg cell (a short
                // detour beside an m2 pond or step) the labelled water its path reaches next
                // within 150 m (ruling, fix round 2); else none (judged against itself).
                let id = if body[j] > 1 {
                    Some(body[j])
                } else {
                    let (mut c, mut walked, mut found) = (j, 0.0, None);
                    while walked <= TRIBUTARY_M {
                        if surf[c].is_finite() && body[c] > 1 {
                            found = Some(body[c]);
                            break;
                        }
                        let dc = flow[c] & 15;
                        let Some(n) = down(c) else { break };
                        walked += step_len(dc) * s.spacing_m;
                        c = n;
                    }
                    found
                };
                let fails_gap = match id {
                    Some(b) if body[p] == b => false,
                    Some(b) => {
                        let (px, py) = win.world_xy(p);
                        let r = (px as f64 - x as f64).hypot(py as f64 - y as f64);
                        zl < own_min(j, r + 1.0, b) - THALWEG_M
                    }
                    None => zl < dj - THALWEG_M,
                };
                // Only a gap the excuses or the channel test decide is counted below.
                let wet_p = body[p] > 1 && surf[p].is_finite();
                let channel_p = wet_p || (flow[p] & 16 != 0 && q[p] >= thr);
                let mut beside = fails_gap;
                if beside && let Some(b) = id {
                    // Either may be the tributary: p's channel (a river thalweg or labelled
                    // water, not a dry hollow) joining this river, or this one joining p's.
                    if channel_p && joins(p, b) {
                        (beside, t.tributary_excused[0]) = (false, t.tributary_excused[0] + 1);
                    } else if wet_p && joins(j, body[p]) {
                        (beside, t.tributary_excused[1]) = (false, t.tributary_excused[1] + 1);
                    }
                }
                // Ruling (refines the spec's "lowest raw ground"): the lower cell is a second
                // thalweg only where it or a cell within 2 cells carries a channel (m3 Q at the
                // river threshold, not this river's own cells); dry lower ground across a valley
                // or fan is not.
                // Gate 1 ruling: the tributary excuses apply to that channel cell too (water
                // this river joins, or a thalweg joining this river).
                if beside {
                    let (px, py) = win.world_xy(p);
                    let own_cell = |c: usize| {
                        let wet = surf[c].is_finite();
                        c == j
                            || (wet && id == Some(body[c]))
                            || (wet && body[c] > 1 && joins(j, body[c]))
                            || (flow[c] & 16 != 0 && id.is_some_and(|b| joins(c, b)))
                    };
                    // Fix round 2: written water of another body (body > 1; 0 none, 1 sea) is
                    // not a second channel; an unwritten m3 channel is.
                    let written = |c: usize| surf[c].is_finite() && body[c] > 1;
                    beside = (-2i64..=2).any(|oy| {
                        (-2i64..=2).any(|ox| {
                            at_i(px as i64 + ox, py as i64 + oy)
                                .is_some_and(|c| q[c] >= thr && !own_cell(c) && !written(c))
                        })
                    });
                }
                if beside {
                    t.fail("thalweg_beside", x, y);
                }
            }
            let near = (20.0 / s.spacing_m).ceil() as i64;
            let beside = (-near..=near).any(|oy| {
                (-near..=near).any(|ox| {
                    ((ox * ox + oy * oy) as f64).sqrt() * s.spacing_m <= 20.0
                        && at_i(x as i64 + ox, y as i64 + oy)
                            .is_some_and(|c| thal(c) && body[c] != body[j] && body[c] > 1)
                })
            });
            if beside {
                t.beside_m += step;
            }
        }
    }
    // Exits in this tile (every full node): the outflow's first cell, falls, steps, rims.
    for e in exits {
        let lv = e.level;
        let [inside, outside] = e.outlet;
        let (bx, by) = world.xy(outside);
        if !core.contains(bx, by) {
            continue;
        }
        let b = at(bx, by).expect("in");
        let (ix, iy) = world.xy(inside);
        // The cell of the spill edge that carries the saddle shows the level (C1): the inside
        // cell is lake at the level, so the outside one may only stand above it by the 1 cm.
        if e.lake.is_some() {
            let first = at(ix, iy).map_or(shown(b), |a| shown(a).max(shown(b)));
            if (first - lv).abs() > SILL_M {
                t.fail("lake_exit_surface", bx, by);
            }
        }
        let (mut prev, mut prev_raw) = (lv, at(ix, iy).map_or(raw[b], |a| raw[a]));
        let (mut cx, mut cy, mut walked) = (bx, by, 0.0f64);
        let mut fall = [f32::NAN; 3];
        while walked <= FALL_M[2] {
            let Some(c) = at(cx, cy) else { break };
            let drop = prev - dsurf(c);
            if drop > DAM_STEP_M {
                t.drops_any.push((e.key, e.lake, [cx, cy], drop));
                for (i, spot) in spots.iter().enumerate() {
                    let Some([sx, sy]) = spot.cell else { continue };
                    let d = (sx.abs_diff(cx) as f64).hypot(sy.abs_diff(cy) as f64) * s.spacing_m;
                    if d <= SPOT_M {
                        let raw_drop = prev_raw - raw[c];
                        t.spot_drops
                            .push((i, e.key, e.lake, [cx, cy], drop, raw_drop));
                    }
                }
                if e.lake.is_some() && prev_raw - raw[c] < drop - SILL_M {
                    t.fail("dam_steps", cx, cy);
                }
            }
            (prev, prev_raw) = (dsurf(c), raw[c]);
            for (f, &reach) in fall.iter_mut().zip(&FALL_M) {
                if f.is_nan() && walked >= reach {
                    *f = lv - prev;
                }
            }
            let Some(n) = receiver(cx, cy, c) else { break };
            walked += if n.0 != cx && n.1 != cy {
                std::f64::consts::SQRT_2
            } else {
                1.0
            } * s.spacing_m;
            (cx, cy) = n;
        }
        // A path that ends sooner: its fall to where it ends.
        for f in &mut fall {
            if f.is_nan() {
                *f = lv - prev;
            }
        }
        t.falls.push(Fall {
            key: e.key,
            lake: e.lake,
            cell: [bx, by],
            m: fall,
        });
        // Rim: dry cells within 2 cells of the exit on its outflow side.
        let out_dir = receiver(bx, by, b)
            .map(|(rx, ry)| (rx as i64 - bx as i64, ry as i64 - by as i64))
            .unwrap_or((bx as i64 - ix as i64, by as i64 - iy as i64));
        for oy in -2i64..=2 {
            for ox in -2i64..=2 {
                if ox * out_dir.0 + oy * out_dir.1 < 0 {
                    continue;
                }
                let (px, py) = (bx as i64 + ox, by as i64 + oy);
                let Some(p) = at_i(px, py) else {
                    continue;
                };
                if surf[p].is_nan() && z[p] > lv + DAM_RIM_M {
                    let cell = [px as u64, py as u64];
                    t.rims_any
                        .push((e.key, e.lake, cell, z[p] - lv, raw[p] - lv));
                    if e.lake.is_some() && raw[p] <= lv + DAM_RIM_M {
                        t.fail("dam_rims", cell[0], cell[1]);
                    }
                }
            }
        }
    }
    t.runs.sort_by(|a, b| b.length_m.total_cmp(&a.length_m));
    t.runs.truncate(20);
    t.unlabelled
        .3
        .sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    t.unlabelled.3.truncate(10);
    Ok(t)
}

pub(crate) struct CheckExtra {
    /// Cells on flow cycles in the third model run (the model refuses cycles, so 0 here).
    pub cycles: u64,
    pub bodies: usize,
    /// Closed lakes covering fewer cells than their supply needs.
    pub lake_short: u64,
    /// The lakes, for the lowest-saddle check over all tiles.
    pub lakes: LakeInfo,
    pub cell_km2: f64,
    /// The review spots of `dam_drops_at_spots`.
    pub spots: Vec<Spot>,
}

/// The largest `n` falls by reach `i` (0: 5 m, 1: 10 m, 2: 50 m), as JSON.
fn falls_json(falls: &mut [Fall], i: usize, n: usize) -> Value {
    falls.sort_by(|a, b| b.m[i].total_cmp(&a.m[i]).then(a.key.cmp(&b.key)));
    json!(
        falls
            .iter()
            .take(n)
            .map(
                |f| json!({"node": f.key, "lake": f.lake, "exit_cell": f.cell,
                            "fall_5m": f.m[0], "fall_10m": f.m[1], "fall_50m": f.m[2]})
            )
            .collect::<Vec<_>>()
    )
}

/// The report: checks (all must be 0) and figures; `passed`.
pub(crate) fn report(
    tiles: Vec<Tile>,
    extra: CheckExtra,
    mut figures: Map<String, Value>,
) -> (Value, bool) {
    let cell_km2 = extra.cell_km2;
    let mut fails: BTreeMap<&str, u64> = [
        "drainage_rises",
        "drainage_stuck",
        "bed_rises",
        "lake_not_flat",
        "lake_leaks",
        "lake_exit_surface",
        "dam_steps",
        "dam_rims",
        "thalweg_beside",
        "walls",
        "change_max",
        "wet_not_above_ground",
        "wet_without_surface",
        "surface_without_body",
        "lake_level_json",
        "wet_unknown_body",
    ]
    .into_iter()
    .map(|k| (k, 0))
    .collect();
    let mut examples: BTreeMap<&str, Vec<[u64; 2]>> = BTreeMap::new();
    let (mut hist, mut hist_changed) = (vec![0u64; BINS], vec![0u64; BINS]);
    let (mut planar, mut straight, mut river, mut beside, mut ponds) =
        ([0u64; 2], 0.0, 0.0, 0.0, 0u64);
    let (mut max_change, mut max_q) = (0.0f32, 0.0f32);
    let mut falls = Vec::new();
    let mut saddle: BTreeMap<u32, f32> = BTreeMap::new();
    let (mut runs, mut drops_any, mut rims_any) = (Vec::new(), Vec::new(), Vec::new());
    let (mut agree, mut closed_ends, mut pond_run_cells) = ([0u64; 4], 0u64, 0u64);
    let mut excused = [0u64; 2];
    let mut bank = (0u64, 0f32, Vec::new());
    let mut class_cells: BTreeMap<u8, u64> = BTreeMap::new();
    let mut class_lakes: BTreeSet<(u8, u32)> = BTreeSet::new();
    let mut lake_gap = None;
    let mut pond_lost = (0u64, None);
    let mut spot_drops: Vec<SpotDrop> = Vec::new();
    let mut unlabelled = (0u64, 0u64, 0u64, Vec::new());
    for t in tiles {
        spot_drops.extend(t.spot_drops.iter().copied());
        unlabelled.0 += t.unlabelled.0;
        unlabelled.1 += t.unlabelled.1;
        unlabelled.2 += t.unlabelled.2;
        unlabelled.3.extend(t.unlabelled.3.iter().copied());
        lake_gap = wider_gap(lake_gap, t.lake_m3_gap);
        pond_lost = (
            pond_lost.0 + t.pond_lost.0,
            deeper_drop(pond_lost.1, t.pond_lost.1),
        );
        for (id, s) in &t.saddle {
            let e = saddle.entry(*id).or_insert(f32::INFINITY);
            *e = e.min(*s);
        }
        runs.extend(t.runs.iter().copied());
        drops_any.extend(t.drops_any.iter().copied());
        rims_any.extend(t.rims_any.iter().copied());
        for (a, b) in agree.iter_mut().zip(t.agree) {
            *a += b;
        }
        closed_ends += t.closed_ends;
        excused[0] += t.tributary_excused[0];
        excused[1] += t.tributary_excused[1];
        pond_run_cells += t.pond_run_cells;
        bank.0 += t.bank_steps.0;
        bank.1 = bank.1.max(t.bank_steps.1);
        bank.2
            .extend(t.bank_steps.2.into_iter().take(5 - bank.2.len().min(5)));
        for (c, n) in t.class_cells {
            *class_cells.entry(c).or_insert(0) += n;
        }
        class_lakes.extend(t.class_lakes);
        for (k, v) in t.fails {
            *fails.entry(k).or_insert(0) += v;
        }
        for (k, v) in t.examples {
            let e = examples.entry(k).or_default();
            e.extend(v.into_iter().take(5 - e.len().min(5)));
        }
        for (h, v) in hist.iter_mut().zip(t.hist) {
            *h += v;
        }
        for (h, v) in hist_changed.iter_mut().zip(t.hist_changed) {
            *h += v;
        }
        (planar[0], planar[1]) = (planar[0] + t.planar[0], planar[1] + t.planar[1]);
        (straight, river, beside, ponds) = (
            straight + t.straight_m,
            river + t.river_m,
            beside + t.beside_m,
            ponds + t.pond_cells,
        );
        max_change = max_change.max(t.max_change);
        max_q = max_q.max(t.max_q);
        falls.extend(t.falls);
    }
    let p99 = p99_m(&hist);
    fails.insert("drainage_cycles", extra.cycles);
    fails.insert("lake_balance", extra.lake_short);
    fails.insert(
        "straight_share",
        u64::from(river > 0.0 && straight / river > STRAIGHT_SHARE),
    );
    fails.insert("facets", u64::from(planar[1] > planar[0]));
    fails.insert("change_p99", u64::from(p99 > 1.0));
    fails.insert("export_bodies", u64::from(extra.bodies > BODY_BUDGET));
    // A full lake stands at its lowest saddle (C1).
    let off_sill: Vec<u32> = extra
        .lakes
        .iter()
        .filter(|(id, l)| l.1 && saddle.get(id).is_none_or(|&s| (s - l.0).abs() > SILL_M))
        .map(|(&id, _)| id)
        .collect();
    fails.insert("lake_off_sill", off_sill.len() as u64);
    let agreement = if agree[0] + agree[2] == 0 {
        1.0
    } else {
        (agree[1] + agree[3]) as f64 / (agree[0] + agree[2]) as f64
    };
    fails.insert("label_agreement", u64::from(agreement < AGREEMENT));
    runs.sort_by(|a, b| b.length_m.total_cmp(&a.length_m));
    drops_any.sort_by(|a, b| b.3.total_cmp(&a.3));
    rims_any.sort_by(|a, b| b.3.total_cmp(&a.3));
    let per_class: Map<String, Value> = class_cells
        .iter()
        .map(|(&c, &n)| {
            let km2 = n as f64 * cell_km2;
            let lakes = class_lakes.iter().filter(|p| p.0 == c).count();
            (
                c.to_string(),
                json!({"km2": km2, "lakes": lakes, "lakes_per_km2": lakes as f64 / km2}),
            )
        })
        .collect();
    figures.insert(
        "lakes_off_sill".into(),
        json!(off_sill.iter().take(20).collect::<Vec<_>>()),
    );
    figures.insert("label_agreement".into(), json!(agreement));
    figures.insert(
        "label_agreement_counts".into(),
        json!({"written": agree[0], "written_matched": agree[1], "final": agree[2], "final_matched": agree[3]}),
    );
    figures.insert(
        "longest_straight_runs".into(),
        json!(
            runs.iter()
                .take(20)
                .map(
                    |r| json!({"length_m": r.length_m, "from": r.from, "to": r.to,
                                "pond_share": r.pond, "raw_thalweg_share": r.raw,
                                "merged_runs": r.members})
                )
                .collect::<Vec<_>>()
        ),
    );
    figures.insert("straight_run_cells_in_ponds".into(), json!(pond_run_cells));
    figures.insert(
        "dam_drops_any".into(),
        json!({"count": drops_any.len(), "largest": drops_any.iter().take(50)
            .map(|d| json!({"node": d.0, "lake": d.1, "cell": d.2, "drop_m": d.3}))
            .collect::<Vec<_>>()}),
    );
    figures.insert(
        "dam_rims_any".into(),
        json!({"count": rims_any.len(), "highest": rims_any.iter().take(50)
            .map(|d| json!({"node": d.0, "lake": d.1, "cell": d.2,
                            "after_above_level_m": d.3, "before_above_level_m": d.4}))
            .collect::<Vec<_>>()}),
    );
    // Per review spot: the exit-walk drops within SPOT_M, the 10 largest with their raw drop.
    spot_drops.sort_by(|a, b| b.4.total_cmp(&a.4).then(a.3.cmp(&b.3)));
    figures.insert(
        "dam_drops_at_spots".into(),
        json!(extra.spots.iter().enumerate().map(|(i, sp)| {
            let mine: Vec<&SpotDrop> = spot_drops.iter().filter(|d| d.0 == i).collect();
            json!({"spot_en": sp.en, "cell": sp.cell, "within_m": SPOT_M, "count": mine.len(),
                   "largest": mine.iter().take(10).map(|d| json!({
                       "node": d.1, "lake": d.2, "cell": d.3, "drop_m": d.4, "raw_drop_m": d.5,
                   })).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()),
    );
    unlabelled
        .3
        .sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    figures.insert(
        "unlabelled_main_river_cells".into(),
        json!({"cells": unlabelled.0, "runs_10_cells": unlabelled.1,
               "run_cells": unlabelled.2,
               "largest": unlabelled.3.iter().take(10)
                   .map(|u| json!({"q_km2": u.0, "cell": u.1})).collect::<Vec<_>>()}),
    );
    figures.insert("big_rivers_into_closed_nodes".into(), json!(closed_ends));
    figures.insert(
        "closed_lake_m3_level_gap".into(),
        json!(lake_gap.map(|(m, id)| json!({"max_m": m, "lake": id}))),
    );
    figures.insert(
        "m2_pond_cells_lost".into(),
        json!({"cells": pond_lost.0, "max_drop": pond_lost.1.map(|(m, node, cell)| {
            json!({"m": m, "m2_node": node, "cell": cell})
        })}),
    );
    figures.insert(
        "thalweg_beside_tributary_excused".into(),
        json!({"channel_joins_river": excused[0], "river_joins_water": excused[1]}),
    );
    figures.insert(
        "straight_share".into(),
        json!(if river > 0.0 { straight / river } else { 0.0 }),
    );
    figures.insert("river_km_checked".into(), json!(river / 1000.0));
    figures.insert(
        "river_km_beside_another_within_20m".into(),
        json!(beside / 1000.0),
    );
    figures.insert("planar_windows_raw_vs_result".into(), json!(planar));
    figures.insert("change_p99_m".into(), json!(p99));
    figures.insert(
        "change_p99_changed_cells_m".into(),
        json!(p99_m(&hist_changed)),
    );
    figures.insert("change_max_m".into(), json!(max_change));
    figures.insert("max_discharge_km2".into(), json!(max_q));
    figures.insert("pond_cells".into(), json!(ponds));
    figures.insert("pond_km2".into(), json!(ponds as f64 * cell_km2));
    figures.insert(
        "bank_steps_over_0_5m".into(),
        json!({"count": bank.0, "highest_m": bank.1, "cells": bank.2}),
    );
    figures.insert("labelled_lakes_per_class".into(), Value::Object(per_class));
    // Every full node, and listed lakes apart (small hillside ponds dominate the first lists).
    let mut lake_falls: Vec<Fall> = falls.iter().filter(|f| f.lake.is_some()).copied().collect();
    figures.insert(
        "exit_falls".into(),
        json!({
            "exits": falls.len(),
            "largest_5m": falls_json(&mut falls, 0, 10),
            "largest_10m": falls_json(&mut falls, 1, 10),
            "largest_50m": falls_json(&mut falls, 2, 10),
            "lakes": {
                "exits": lake_falls.len(),
                "largest_5m": falls_json(&mut lake_falls, 0, 10),
                "largest_10m": falls_json(&mut lake_falls, 1, 10),
                "largest_50m": falls_json(&mut lake_falls, 2, 10),
            },
        }),
    );
    // Owner ruling 2026-11-06: straight and parallel rivers from the terrain before hydrology
    // are accepted, so these are figures (counts kept; examples stay in `examples`).
    for name in FIGURE_ONLY {
        figures.insert(
            format!("{name}_count"),
            json!(fails.remove(name).unwrap_or(0)),
        );
    }
    let failures: Vec<&str> = fails
        .iter()
        .filter(|(_, v)| **v > 0)
        .map(|(k, _)| *k)
        .collect();
    let passed = failures.is_empty();
    (
        json!({
            "schema_version": 1, "kind": "water_report", "passed": passed, "failures": failures,
            "checks": fails, "examples": examples, "figures": figures,
        }),
        passed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::MIN_DEPTH_M;
    use crate::world::water::geom::World;
    use crate::world::water::testing::setup;

    /// A hand-built finished run on a `w` × `h` world: every layer dry and unrouted (dry basin
    /// bit set, so nothing is stuck) until a test sets it.
    struct Case {
        world: World,
        raw: Vec<f32>,
        z: Vec<f32>,
        surf: Vec<f32>,
        body: Vec<u32>,
        level: Vec<f32>,
        node: Vec<u32>,
        flow: Vec<u8>,
        q: Vec<f32>,
        fw: Vec<u8>,
        /// m2 nodes and the ground before channels (bed_rises).
        n2: Vec<u32>,
        /// m2 levels (m2_pond_cells_lost).
        l2: Vec<f32>,
        bz: Vec<f32>,
        lakes: LakeInfo,
        /// bodies.json lake and sea levels (`lake_level_json`).
        levels: Levels,
        /// Body ids bodies.json leaves out (`wet_unknown_body`).
        unlisted: BTreeSet<u32>,
        exits: Vec<Exit>,
        spots: Vec<Spot>,
    }

    impl Case {
        fn new(w: u64, h: u64, ground: impl Fn(u64, u64) -> f32) -> Self {
            let world = World { w, h };
            let n = (w * h) as usize;
            let z: Vec<f32> = (0..n as u64).map(|i| ground(i % w, i / w)).collect();
            Case {
                world,
                raw: z.clone(),
                bz: z.clone(),
                z,
                surf: vec![f32::NAN; n],
                body: vec![0; n],
                level: vec![f32::NAN; n],
                node: vec![0; n],
                flow: vec![32; n],
                q: vec![0.0; n],
                fw: vec![0; n],
                n2: vec![0; n],
                l2: vec![f32::NAN; n],
                lakes: LakeInfo::new(),
                levels: Levels::new(),
                unlisted: BTreeSet::new(),
                exits: Vec::new(),
                spots: Vec::new(),
            }
        }
        fn k(&self, x: u64, y: u64) -> usize {
            (y * self.world.w + x) as usize
        }
        /// bodies.json as the export reads it: every labelled id but `unlisted`.
        fn listed(&self) -> Listed {
            Listed {
                levels: self.levels.clone(),
                ids: self
                    .body
                    .iter()
                    .copied()
                    .filter(|b| *b != 0 && !self.unlisted.contains(b))
                    .collect(),
            }
        }
        /// The D8 byte from (x, y) to its neighbour (nx, ny).
        fn byte(a: (u64, u64), b: (u64, u64)) -> u8 {
            OFFSETS
                .iter()
                .position(|&(dy, dx)| {
                    (a.0 as i64 + dx, a.1 as i64 + dy) == (b.0 as i64, b.1 as i64)
                })
                .expect("a D8 step") as u8
                + 1
        }
        /// Routes `path` cell to cell with `bits` (16: thalweg) and discharge `q`.
        fn route(&mut self, path: &[(u64, u64)], bits: u8, q: f32) {
            for w in path.windows(2) {
                let k = self.k(w[0].0, w[0].1);
                self.flow[k] = Self::byte(w[0], w[1]) | bits;
                self.q[k] = q;
            }
        }
        /// A written river: routed thalweg (m2 and m3 alike), wet at the ground, body `id`.
        fn river(&mut self, path: &[(u64, u64)], q: f32, id: u32) {
            self.route(path, 16, q);
            for &(x, y) in &path[..path.len() - 1] {
                let k = self.k(x, y);
                (self.surf[k], self.body[k], self.fw[k]) = (self.z[k], id, self.flow[k]);
            }
        }
        /// The rivers run off every written (m2) thalweg.
        fn m3_only(&mut self) {
            self.fw.fill(0);
        }
        fn tile(&self) -> Tile {
            check_tile(
                &setup(self.world, 4096, 0.05),
                &self.layers(),
                Rect::whole(self.world),
                &self.lakes,
                &self.listed(),
                &self.exits,
                &self.spots,
            )
            .expect("check")
        }
        /// The report's check counts over tiles of `tile` cells.
        fn checks_at(&self, tile: u64) -> Value {
            let (s, l) = (setup(self.world, tile, 0.05), self.layers());
            let tiles = s
                .tiles()
                .into_iter()
                .map(|core| {
                    check_tile(
                        &s,
                        &l,
                        core,
                        &self.lakes,
                        &self.listed(),
                        &self.exits,
                        &self.spots,
                    )
                    .expect("check")
                })
                .collect();
            // With the figure-only counts (owner ruling 2026-11-06) under their check names.
            let r = report(tiles, self.extra(), Map::new()).0;
            let mut checks = r["checks"].clone();
            for name in FIGURE_ONLY {
                checks[name] = r["figures"][format!("{name}_count")].clone();
            }
            checks
        }
        fn extra(&self) -> CheckExtra {
            CheckExtra {
                cycles: 0,
                bodies: 0,
                lake_short: 0,
                lakes: self.lakes.clone(),
                cell_km2: 1e-6,
                spots: self.spots.clone(),
            }
        }
        fn layers(&self) -> CheckLayers {
            let w = self.world;
            CheckLayers {
                raw: Layer::memory(w, self.raw.clone()),
                ground: Layer::memory(w, self.z.clone()),
                surface: Layer::memory(w, self.surf.clone()),
                body: Layer::memory(w, self.body.clone()),
                level: Layer::memory(w, self.level.clone()),
                node: Layer::memory(w, self.node.clone()),
                flow: Layer::memory(w, self.flow.clone()),
                discharge: Layer::memory(w, self.q.clone()),
                flow_written: Layer::memory(w, self.fw.clone()),
                node_written: Layer::memory(w, self.n2.clone()),
                level_written: Layer::memory(w, self.l2.clone()),
                breached: Layer::memory(w, self.bz.clone()),
            }
        }
        fn report(&self) -> Value {
            report(vec![self.tile()], self.extra(), Map::new()).0
        }
    }

    fn fails(t: &Tile, name: &str) -> u64 {
        t.fails.get(name).copied().unwrap_or(0)
    }

    /// A plane falling east 0.01 and south 0.1 per cell.
    fn plane(x: u64, y: u64) -> f32 {
        100.0 - 0.01 * x as f32 - 0.1 * y as f32
    }

    #[test]
    fn plane_fit_finds_facets_only_on_steep_planes() {
        let steep: Vec<f32> = (0..25).map(|i| (i % 5) as f32 * 0.2).collect();
        assert!(planar(&steep, 1.0), "a 11° plane");
        let flat: Vec<f32> = vec![1.0; 25];
        assert!(!planar(&flat, 1.0), "flat is not a facet");
        let rough: Vec<f32> = (0..25)
            .map(|i| (i % 5) as f32 * 0.2 + ((i * 7) % 3) as f32 * 0.1)
            .collect();
        assert!(!planar(&rough, 1.0));
    }

    #[test]
    fn horn_slope_matches_a_plane() {
        let z: Vec<f32> = (0..9).map(|i| (i % 3) as f32).collect();
        assert!((horn_deg(&z, 3, 1, 1, 1.0) - 45.0).abs() < 1e-3);
    }

    /// A river of discharge 2 km² along `path` on a `w` × `h` plane (raw `raw`, else the
    /// plane), and optionally a second flow line of 0.05 km² along row `beside` (x 10..240,
    /// gaps every `gap` cells) that never joins it.
    fn river_on(
        w: u64,
        h: u64,
        path: &[(u64, u64)],
        beside: Option<(u64, u64)>,
        raw: Option<Vec<f32>>,
    ) -> Tile {
        river_beside(w, h, path, beside, raw, 0.05)
    }

    fn river_beside(
        w: u64,
        h: u64,
        path: &[(u64, u64)],
        beside: Option<(u64, u64)>,
        raw: Option<Vec<f32>>,
        q: f32,
    ) -> Tile {
        let mut c = Case::new(w, h, plane);
        c.river(path, 2.0, 5);
        if let Some((row, gap)) = beside {
            for x in 10..240 {
                let k = c.k(x, row);
                (c.flow[k], c.q[k]) = (if gap > 0 && x % gap == 0 { 0 } else { 4 }, q);
            }
        }
        if let Some(r) = raw {
            c.raw = r;
        }
        c.tile()
    }

    /// A straight river along row 10, x 10..240.
    fn river_case(beside: Option<u64>) -> Tile {
        let path: Vec<(u64, u64)> = (10..241).map(|x| (x, 10)).collect();
        river_on(260, 30, &path, beside.map(|r| (r, 0)), None)
    }

    #[test]
    fn a_long_straight_river_and_a_parallel_channel_fail() {
        let alone = river_case(None);
        assert!(
            fails(&alone, "straight_runs") > 0,
            "a 230 m run on a tilted plane"
        );
        assert_eq!(fails(&alone, "parallel_rivers"), 0);
        let doubled = river_case(Some(15));
        assert!(
            fails(&doubled, "parallel_rivers") > 0,
            "a second channel 5 m away"
        );
        assert_eq!(
            fails(&river_case(Some(25)), "parallel_rivers"),
            0,
            "15 m away is fine"
        );
        // A hillslope line below the river threshold (0.03 km²) 5 m away is ordinary flow.
        let path: Vec<(u64, u64)> = (10..241).map(|x| (x, 10)).collect();
        let rill = river_beside(260, 30, &path, Some((15, 0)), None, 0.03);
        assert_eq!(fails(&rill, "parallel_rivers"), 0);
    }

    /// A doubled channel with a 1 m gap every 10 m is still one doubled channel (R29: gaps up
    /// to 5 m are bridged).
    #[test]
    fn a_gappy_doubled_channel_fails() {
        let path: Vec<(u64, u64)> = (10..241).map(|x| (x, 10)).collect();
        let t = river_on(260, 30, &path, Some((15, 10)), None);
        assert_eq!(fails(&t, "parallel_rivers"), 1, "{:?}", t.examples);
    }

    /// A diagonal river and a parallel diagonal flow line 3 cells east of it (an odd x − y
    /// offset, about 2.1 m across): sampled along the normal only, it is never seen.
    #[test]
    fn a_doubled_channel_beside_a_diagonal_river_fails() {
        let mut c = Case::new(200, 200, |x, y| 100.0 - 0.05 * (x + y) as f32);
        let path: Vec<(u64, u64)> = (5..185).map(|i| (i, i)).collect();
        c.river(&path, 2.0, 5);
        let line: Vec<(u64, u64)> = (5..180).map(|i| (i + 3, i)).collect();
        c.route(&line, 0, 0.05);
        let t = c.tile();
        assert_eq!(fails(&t, "parallel_rivers"), 1, "{:?}", t.examples);
    }

    /// A digital line at 1:10 (no D8 direction for more than 10 cells) is as straight as an
    /// axis-aligned one (R30 at any angle); a meander of the same length is not.
    #[test]
    fn straightness_is_measured_at_any_angle() {
        let mut slanted = vec![(5u64, 2u64)];
        for i in 1..=220u64 {
            slanted.push((5 + i, 2 + i / 10));
        }
        let t = river_on(260, 30, &slanted, None, None);
        assert_eq!(
            fails(&t, "straight_runs"),
            220,
            "every cell of one 1:10 run of 228 m"
        );
        assert!(t.runs[0].length_m > 220.0, "{:?}", t.runs[0]);
        let t = river_on(260, 30, &wobble(5.0), None, None);
        assert_eq!(fails(&t, "straight_runs"), 0, "{:?}", t.runs.first());
    }

    /// x 5..250 along row 14 with a sine wobble of `amp` cells (wavelength 60), as a D8 path.
    fn wobble(amp: f64) -> Vec<(u64, u64)> {
        let mut path = vec![(5u64, 14u64)];
        for x in 6..250u64 {
            let y = (14.0 + amp * (x as f64 * std::f64::consts::TAU / 60.0).sin()).round() as u64;
            let (px, py) = *path.last().unwrap();
            let mut cy = py;
            while cy != y {
                cy = if y > cy { cy + 1 } else { cy - 1 };
                if cy == y {
                    break;
                }
                path.push((px, cy));
            }
            path.push((x, y));
        }
        path
    }

    /// The 1 m chord tolerance: a river wobbling ±0.5 m over 240 m is a straight line and
    /// fails; one wobbling ±1.5 m does not count as straight.
    #[test]
    fn the_chord_tolerance_is_one_metre() {
        let path = wobble(0.5);
        let t = river_on(260, 30, &path, None, None);
        assert_eq!(
            fails(&t, "straight_runs"),
            path.len() as u64 - 1,
            "every river cell: {:?}",
            t.runs.first()
        );
        let t = river_on(260, 30, &wobble(1.5), None, None);
        assert_eq!(fails(&t, "straight_runs"), 0, "{:?}", t.runs.first());
    }

    /// A 1.4 km straight river across many tile seams (runs up to 400 cells, longer than the
    /// old 160-cell halo): every check counts alike at every tiling, and `straight_runs`
    /// counts each covered river cell once.
    #[test]
    fn a_straight_run_across_tile_seams_counts_once() {
        let mut c = Case::new(1500, 24, plane);
        let path: Vec<(u64, u64)> = (10..1440).map(|x| (x, 10)).collect();
        c.river(&path, 2.0, 5);
        // A raw trench under x 400..1300 exempts the runs inside it; a run cut short at a
        // window edge would see another raw share.
        for x in 400..1300 {
            let k = c.k(x, 10);
            c.raw[k] -= 1.0;
        }
        let whole = c.checks_at(4096);
        let n = whole["straight_runs"].as_u64().unwrap();
        assert!(n > 600 && n < 1429, "{n} cells");
        // Owner ruling 2026-11-06: a figure, not a failure.
        let r = c.report();
        assert_eq!(r["figures"]["straight_runs_count"], n, "{}", r["figures"]);
        assert!(r["checks"].get("straight_runs").is_none());
        assert!(r["checks"].get("straight_share").is_none());
        assert!(
            r["examples"]["straight_runs"]
                .as_array()
                .is_some_and(|e| !e.is_empty())
        );
        let failures = r["failures"].as_array().unwrap();
        assert!(
            !failures
                .iter()
                .any(|f| f == "straight_runs" || f == "straight_share"),
            "{failures:?}"
        );
        for tile in [128, 200, 700] {
            assert_eq!(c.checks_at(tile), whole, "tile {tile}");
        }
    }

    /// A long, gentle arc: many maximal runs overlap along it; their cells count once each.
    #[test]
    fn overlapping_straight_runs_count_once() {
        let mut path = vec![(5u64, 5u64)];
        for x in 6..415u64 {
            let y = (5.0 + (x as f64 - 5.0).powi(2) / 10_000.0).round() as u64;
            let (px, py) = *path.last().unwrap();
            if y > py + 1 || (y > py && px + 1 != x) {
                path.push((px, py + 1));
            }
            path.push((x, y));
        }
        let t = river_on(420, 40, &path, None, None);
        assert!(t.runs.iter().any(|r| r.members > 1), "{:?}", t.runs);
        let cells = fails(&t, "straight_runs");
        assert!(
            cells > 150 && cells < path.len() as u64,
            "{cells} cells: {:?}",
            t.runs
        );
    }

    /// The straight-gorge exemption needs a raw thalweg strictly below the cells across: an
    /// exactly flat raw floor (ties) does not exempt a run, a raw trench does.
    #[test]
    fn a_flat_raw_floor_does_not_exempt_a_straight_run() {
        let path: Vec<(u64, u64)> = (10..241).map(|x| (x, 10)).collect();
        let n = 260 * 30;
        let flat = vec![50.0f32; n];
        assert!(
            fails(
                &river_on(260, 30, &path, None, Some(flat.clone())),
                "straight_runs"
            ) > 0
        );
        let mut trench = flat;
        for x in 0..260 {
            trench[10 * 260 + x] = 49.0;
        }
        assert_eq!(
            fails(
                &river_on(260, 30, &path, None, Some(trench)),
                "straight_runs"
            ),
            0
        );
    }

    #[test]
    fn p99_reads_the_histogram() {
        let mut hist = vec![0u64; BINS];
        hist[0] = 990;
        hist[150] = 10;
        assert_eq!(p99_m(&hist), 0.0);
        hist[150] = 11;
        assert!((p99_m(&hist) - 1.5).abs() < 0.011);
    }

    /// A cell draining into water standing above it rises; into lower water it does not.
    #[test]
    fn drainage_rises_where_a_receiver_stands_higher() {
        let mut c = Case::new(9, 9, plane);
        c.route(&[(3, 4), (4, 4)], 0, 0.0);
        let r = c.k(4, 4);
        (c.surf[r], c.body[r]) = (c.z[c.k(3, 4)] + 0.3, 7);
        let t = c.tile();
        assert_eq!(fails(&t, "drainage_rises"), 1);
        assert_eq!(t.examples["drainage_rises"], vec![[3, 4]]);
        c.surf[r] = c.z[r];
        assert_eq!(fails(&c.tile(), "drainage_rises"), 0);
    }

    /// A cell without a receiver is stuck when dry; as water (a wetted pond) it is not.
    #[test]
    fn a_dry_cell_without_a_receiver_is_stuck_but_water_is_not() {
        let mut c = Case::new(9, 9, plane);
        let k = c.k(4, 4);
        c.flow[k] = 0;
        assert_eq!(fails(&c.tile(), "drainage_stuck"), 1);
        (c.surf[k], c.body[k]) = (c.z[k] + 0.2, 9);
        assert_eq!(
            fails(&c.tile(), "drainage_stuck"),
            0,
            "a wetted pond is water"
        );
    }

    /// A 3 × 3 lake at level 10 in a 9 × 9 world of ground `rim` around a 9 m floor.
    fn lake_case(full: bool, rim: f32) -> Case {
        let mut c = Case::new(9, 9, |x, y| {
            if (3..6).contains(&x) && (3..6).contains(&y) {
                9.0
            } else {
                rim
            }
        });
        for y in 3..6 {
            for x in 3..6 {
                let k = c.k(x, y);
                (c.surf[k], c.body[k], c.node[k]) = (10.0, 3, 2);
                c.flow[k] = 0;
            }
        }
        // The outlet: east of (5, 4).
        c.lakes
            .insert(3, (10.0, full, [c.k(5, 4) as u64, c.k(6, 4) as u64], 0));
        c
    }

    #[test]
    fn lake_leaks_spare_the_outlet_and_catch_closed_lakes() {
        // A full lake whose outlet cell lies 0.5 m below the level: the outlet is excused.
        let mut c = lake_case(true, 10.5);
        let o = c.k(6, 4);
        c.z[o] = 9.5;
        assert_eq!(fails(&c.tile(), "lake_leaks"), 0);
        // The same cell beside a closed lake is a leak, seen from each of its 3 lake cells.
        c.lakes.get_mut(&3).unwrap().1 = false;
        assert_eq!(fails(&c.tile(), "lake_leaks"), 3);
    }

    /// A lake cell whose ground reaches the level holds no water: shore, never a leak.
    #[test]
    fn a_zero_depth_shore_does_not_leak() {
        let mut c = lake_case(true, 10.5);
        // (5, 3) is wet at depth 0; outside it, (6, 2) lies 2 m below the level.
        let k = c.k(5, 3);
        c.z[k] = 10.0;
        c.raw[k] = 10.0;
        let low = c.k(6, 2);
        c.z[low] = 8.0;
        let t = c.tile();
        assert_eq!(fails(&t, "lake_leaks"), 0, "{:?}", t.examples);
        // A deep lake cell beside that low cell still leaks.
        let mut c = lake_case(true, 10.5);
        let low = c.k(6, 2);
        c.z[low] = 8.0;
        assert_eq!(fails(&c.tile(), "lake_leaks"), 1);
    }

    #[test]
    fn a_full_lake_off_its_sill_fails() {
        // Every saddle is 0.5 m above the level.
        let c = lake_case(true, 10.5);
        assert_eq!(c.report()["checks"]["lake_off_sill"], 1);
        let mut c = lake_case(true, 10.5);
        let o = c.k(6, 4);
        c.z[o] = 10.0;
        assert_eq!(c.report()["checks"]["lake_off_sill"], 0);
    }

    /// A closed lake labelled at 10 m whose m3 pond balances at 10.31 m: a figure, no failure.
    #[test]
    fn a_closed_lake_reports_its_m3_level_gap() {
        let mut c = lake_case(false, 10.5);
        for y in 3..6 {
            for x in 3..6 {
                let k = c.k(x, y);
                c.level[k] = 10.31;
            }
        }
        let r = c.report();
        assert_eq!(r["checks"]["lake_not_flat"], 0);
        let gap = &r["figures"]["closed_lake_m3_level_gap"];
        assert_eq!(gap["lake"], 3);
        assert!(
            (gap["max_m"].as_f64().unwrap() - 0.31).abs() < 1e-5,
            "{gap}"
        );
    }

    /// An m2 pond at 10 m over x = 2..5 of row 2: m3 keeps x = 2..3 at 9.8 m and loses the
    /// rest (ground 9.5 m and 9.4 m): 2 cells lost, the largest drop 0.6 m at (5, 2).
    #[test]
    fn m2_pond_cells_the_routing_drops_are_reported() {
        let mut c = Case::new(8, 5, |_, _| 9.4);
        for x in 2..6 {
            let k = c.k(x, 2);
            (c.n2[k], c.l2[k]) = (7, 10.0);
            if x < 4 {
                (c.node[k], c.level[k]) = (3, 9.8);
            }
        }
        let k = c.k(4, 2);
        c.z[k] = 9.5;
        let figure = |c: &Case, tile: u64| {
            let (s, l) = (setup(c.world, tile, 0.05), c.layers());
            let tiles = s
                .tiles()
                .into_iter()
                .map(|core| {
                    check_tile(&s, &l, core, &c.lakes, &c.listed(), &c.exits, &c.spots)
                        .expect("check")
                })
                .collect();
            report(tiles, c.extra(), Map::new()).0["figures"]["m2_pond_cells_lost"].clone()
        };
        let f = figure(&c, 8);
        assert_eq!(f["cells"], 2, "{f}");
        assert_eq!(f["max_drop"]["m2_node"], 7);
        assert_eq!(f["max_drop"]["cell"], json!([5, 2]));
        assert!(
            (f["max_drop"]["m"].as_f64().unwrap() - 0.6).abs() < 1e-5,
            "{f}"
        );
        assert_eq!(figure(&c, 3), f, "tiles of 3 alike");
    }

    #[test]
    fn a_lake_exit_above_the_level_fails() {
        let mut c = lake_case(true, 10.5);
        let ex = Exit {
            key: 1,
            level: 10.0,
            outlet: c.lakes[&3].2,
            lake: Some(3),
        };
        c.exits.push(ex);
        // The outside cell at 10.5 m, dry: 0.5 m above the level.
        assert_eq!(fails(&c.tile(), "lake_exit_surface"), 1);
        let o = c.k(6, 4);
        c.z[o] = 10.005;
        assert_eq!(fails(&c.tile(), "lake_exit_surface"), 0);
    }

    /// A 2 m step on the outflow 3 cells below a lake exit: a dam step unless raw has it.
    #[test]
    fn a_dam_step_fails_unless_raw_has_it() {
        let mut c = lake_case(true, 12.0);
        let path: Vec<(u64, u64)> = (6..9).map(|x| (x, 4)).collect();
        for (i, &(x, y)) in path.iter().enumerate() {
            let k = c.k(x, y);
            c.z[k] = if i < 2 { 10.0 - 0.01 * i as f32 } else { 7.9 };
        }
        c.raw = c.z.clone();
        let k = c.k(8, 4);
        c.raw[k] = 9.99;
        c.route(&path, 0, 0.0);
        c.exits.push(Exit {
            key: 1,
            level: 10.0,
            outlet: c.lakes[&3].2,
            lake: Some(3),
        });
        let t = c.tile();
        assert_eq!(fails(&t, "dam_steps"), 1);
        assert_eq!(t.drops_any.len(), 1);
        assert!((t.falls[0].m[0] - 2.1).abs() < 1e-3, "{:?}", t.falls);
        c.raw = c.z.clone();
        let t = c.tile();
        assert_eq!(fails(&t, "dam_steps"), 0, "raw has the step");
        assert_eq!(t.drops_any.len(), 1, "still a figure (R31)");
    }

    /// World rerun 2, lake 323: the exit cell's ground is the level (311.12152 m) and backwater
    /// wets it at its receiver's MIN_DEPTH_M floor. At a 1 cm floor f32 put it 0.010009766 m
    /// above the level, over SILL_M; the floor stays inside the tolerance.
    #[test]
    fn a_min_depth_exit_at_the_level_passes() {
        let lv = 311.121_52_f32;
        let mut c = lake_case(true, 10.5);
        for k in 0..c.z.len() {
            c.z[k] += 301.0;
            if c.surf[k].is_finite() {
                c.surf[k] = lv;
            }
        }
        c.raw = c.z.clone();
        c.lakes.get_mut(&3).unwrap().0 = lv;
        c.levels.insert(3, lv);
        let o = c.k(6, 4);
        (c.z[o], c.raw[o]) = (lv, lv);
        (c.surf[o], c.body[o]) = (lv + MIN_DEPTH_M, 5);
        c.exits.push(Exit {
            key: 1,
            level: lv,
            outlet: c.lakes[&3].2,
            lake: Some(3),
        });
        assert_eq!(fails(&c.tile(), "lake_exit_surface"), 0);
    }

    /// World rerun 2, lake 331: an uncut cell (water MIN_DEPTH_M over raw 317.8263 m) above a
    /// cut cell standing at its raw 316.0113 m, on the exit path. D falls raw's drop plus the
    /// floor; at a 1 cm floor f32 made that 0.010009766 m more than raw, a dam step.
    #[test]
    fn a_min_depth_cell_above_a_cut_step_is_no_dam_step() {
        let mut c = lake_case(true, 12.0);
        for k in 0..c.z.len() {
            c.z[k] += 308.0;
            if c.surf[k].is_finite() {
                c.surf[k] = 318.0;
            }
        }
        c.lakes.get_mut(&3).unwrap().0 = 318.0;
        let path: Vec<(u64, u64)> = (6..9).map(|x| (x, 4)).collect();
        for (&(x, y), raw) in path.iter().zip([317.9f32, 317.8263, 316.0113]) {
            let k = c.k(x, y);
            c.z[k] = raw;
        }
        c.raw = c.z.clone();
        c.route(&path, 16, 1.0);
        for &(x, y) in &path[..2] {
            let k = c.k(x, y);
            (c.surf[k], c.body[k]) = (c.z[k] + MIN_DEPTH_M, 5);
        }
        // The cut cell: bed 0.19 m below raw, surface at raw (R).
        let k = c.k(8, 4);
        (c.surf[k], c.body[k], c.z[k]) = (c.raw[k], 5, c.raw[k] - 0.19);
        c.exits.push(Exit {
            key: 1,
            level: 318.0,
            outlet: c.lakes[&3].2,
            lake: Some(3),
        });
        let t = c.tile();
        assert_eq!(t.drops_any.len(), 1, "a drop over 1.5 m");
        assert_eq!(fails(&t, "dam_steps"), 0);
    }

    #[test]
    fn drops_near_a_spot_are_listed_with_their_raw_drop() {
        let mut c = lake_case(true, 12.0);
        let path: Vec<(u64, u64)> = (6..9).map(|x| (x, 4)).collect();
        for (i, &(x, y)) in path.iter().enumerate() {
            let k = c.k(x, y);
            c.z[k] = if i < 2 { 10.0 - 0.01 * i as f32 } else { 7.9 };
        }
        c.raw = c.z.clone();
        let k = c.k(8, 4);
        c.raw[k] = 9.99;
        c.route(&path, 0, 0.0);
        c.exits.push(Exit {
            key: 1,
            level: 10.0,
            outlet: c.lakes[&3].2,
            lake: Some(3),
        });
        c.spots = vec![
            Spot {
                en: [8.5, 1.5],
                cell: Some([8, 50]),
            },
            Spot {
                en: [0.0, 0.0],
                cell: Some([8, 200]),
            },
            Spot {
                en: [-5.0, 0.0],
                cell: None,
            },
        ];
        let f = c.report()["figures"]["dam_drops_at_spots"].clone();
        assert_eq!(f[0]["count"], 1, "{f}");
        let d = &f[0]["largest"][0];
        assert_eq!(d["cell"], json!([8, 4]));
        assert!((d["drop_m"].as_f64().unwrap() - 2.09).abs() < 1e-3, "{d}");
        assert!(d["raw_drop_m"].as_f64().unwrap().abs() < 1e-3, "{d}");
        assert_eq!(f[0]["spot_en"], json!([8.5, 1.5]));
        assert_eq!(f[1]["count"], 0, "196 m away: {f}");
        assert_eq!(f[2]["count"], 0);
        assert_eq!(f[2]["cell"], Value::Null);
    }

    #[test]
    fn unlabelled_main_river_cells_are_counted() {
        let mut c = Case::new(40, 10, |x, _| 20.0 - 0.1 * x as f32);
        // A dry m3 river (no body, no node) 13 cells along row 2, Q 2 km²: 12 routed cells.
        let main: Vec<(u64, u64)> = (2..15).map(|x| (x, 2)).collect();
        c.route(&main, 16, 2.0);
        // A short dry rill at Q 0.1 (5 routed cells): no main-river cells and no 10-cell run.
        let rill: Vec<(u64, u64)> = (2..8).map(|x| (x, 6)).collect();
        c.route(&rill, 16, 0.1);
        // A written river is labelled.
        let wet: Vec<(u64, u64)> = (20..35).map(|x| (x, 4)).collect();
        c.river(&wet, 2.0, 5);
        let f = c.report()["figures"]["unlabelled_main_river_cells"].clone();
        assert_eq!(f["cells"], 12, "{f}");
        assert_eq!(f["runs_10_cells"], 1, "{f}");
        assert_eq!(f["run_cells"], 12, "{f}");
        assert_eq!(f["largest"][0]["q_km2"], 2.0);
        // Wetting the middle splits it into two runs under 10 cells.
        for x in 7..9 {
            let k = c.k(x, 2);
            (c.surf[k], c.body[k]) = (c.z[k], 6);
        }
        let f = c.report()["figures"]["unlabelled_main_river_cells"].clone();
        assert_eq!(f["cells"], 10, "{f}");
        assert_eq!(f["runs_10_cells"], 0, "{f}");
    }

    /// One thalweg (I3): a river along row 30 of a valley with a lower valley beyond a ridge,
    /// with its own wet cell lowest, or incised 1.5 m (no failure: one-sided).
    #[test]
    fn thalweg_beside_stops_at_the_divide() {
        // A V valley along row 30; a ridge at row 40 (2 m above the floor); a lower valley
        // floor at row 50, 3 m below this one.
        let ground = |x: u64, y: u64| {
            let floor = 20.0 - 0.01 * x as f32;
            let yy = y as f32;
            if y <= 40 {
                floor + 0.2 * (yy - 30.0).abs()
            } else {
                floor + 2.0 - 0.25 * (yy - 40.0)
            }
        };
        let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
        let mut c = Case::new(130, 70, ground);
        c.river(&path, 2.0, 5);
        c.m3_only();
        assert_eq!(
            fails(&c.tile(), "thalweg_beside"),
            0,
            "the ridge is a divide"
        );
        // Its own wet bank cell lowest across: fine.
        let mut c = Case::new(130, 70, ground);
        c.river(&path, 2.0, 5);
        c.m3_only();
        for &(x, _) in &path {
            let k = c.k(x, 31);
            c.raw[k] -= 0.5;
            (c.surf[k], c.body[k]) = (c.z[c.k(x, 30)], 5);
        }
        assert_eq!(fails(&c.tile(), "thalweg_beside"), 0);
        // A river incised 1.5 m below every raw cell across is no failure (one-sided).
        let mut c = Case::new(130, 70, ground);
        c.river(&path, 2.0, 5);
        c.m3_only();
        for &(x, y) in &path {
            let k = c.k(x, y);
            c.surf[k] = c.raw[k] - 1.5;
        }
        assert_eq!(
            fails(&c.tile(), "thalweg_beside"),
            0,
            "too low is not beside"
        );
    }

    /// A true lower parallel channel 20 m away (no divide between), 1.5 m below every own
    /// cell within that distance: beside.
    fn lower_parallel() -> Case {
        let mut c = Case::new(130, 70, |x, y| {
            let floor = 20.0 - 0.01 * x as f32;
            let dy = y as f32 - 30.0;
            if y < 40 {
                floor + 0.05 * dy.abs()
            } else {
                floor + 0.5 - 0.2 * (dy - 10.0).min(10.0)
            }
        });
        let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
        c.river(&path, 2.0, 5);
        let lower: Vec<(u64, u64)> = (10..120).map(|x| (x, 50)).collect();
        c.route(&lower, 16, 0.06);
        c
    }

    #[test]
    fn a_lower_parallel_channel_is_beside() {
        let mut c = lower_parallel();
        c.m3_only();
        let t = c.tile();
        assert!(fails(&t, "thalweg_beside") > 0, "{:?}", t.examples);
    }

    /// A gentle V valley (0.05 per cell across) with a river along row 30, x 10..120, off
    /// every m2 thalweg.
    fn gentle_valley(across: impl Fn(u64, f32) -> f32) -> Case {
        let mut c = Case::new(130, 70, |x, y| {
            20.0 - 0.01 * x as f32 + across(y, 0.05 * (y as f32 - 30.0).abs())
        });
        let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
        c.river(&path, 2.0, 5);
        c.m3_only();
        c
    }

    /// A raw pit 1.5 m below the river, 6–8 m beside it, filled to its spill 0.35 m above the
    /// river: water stands at the pond's level there, so it is no second thalweg.
    #[test]
    fn a_pond_beside_a_river_counts_at_its_level() {
        let pit = |level: bool| {
            let mut c = gentle_valley(|_, v| v);
            for y in 36..39 {
                for x in 50..70 {
                    let (k, floor) = (c.k(x, y), 20.0 - 0.01 * x as f32);
                    // A channel runs through it.
                    (c.raw[k], c.z[k], c.q[k]) = (floor - 1.5, floor - 1.5, 0.06);
                    if level {
                        (c.level[k], c.node[k]) = (floor + 0.35, 2);
                    }
                }
            }
            fails(&c.tile(), "thalweg_beside")
        };
        assert!(pit(false) > 0, "an open hollow 1.5 m below is beside");
        assert_eq!(pit(true), 0);
    }

    /// Lower ground 2 m below the river behind a ridge 8 m away: a ridge 0.7 m above the
    /// river's surface divides; one 0.4 m above does not.
    #[test]
    fn a_half_metre_ridge_divides() {
        let ridge = |h: f32| {
            let mut c = gentle_valley(move |y, v| match y {
                38 => h,
                39..=45 => -2.0,
                _ => v,
            });
            // A channel along the lower ground.
            for x in 0..130 {
                let k = c.k(x, 40);
                c.q[k] = 0.06;
            }
            fails(&c.tile(), "thalweg_beside")
        };
        assert_eq!(ridge(0.7), 0);
        assert!(ridge(0.4) > 0);
    }

    /// A tributary (body 6) running 15 m beside its trunk (body 5) and 1.5 m above it, then
    /// joining it within 150 m: a natural tributary beside its trunk, no second thalweg.
    #[test]
    fn a_tributary_beside_its_trunk_passes() {
        let mut c = gentle_valley(|y, v| if y >= 30 { 0.1 * (y as f32 - 30.0) } else { v });
        let mut trib: Vec<(u64, u64)> = (10..80).map(|x| (x, 45)).collect();
        trib.extend((30..=45).rev().map(|y| (80, y)));
        c.river(&trib, 1.5, 6);
        c.m3_only();
        let t = c.tile();
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
        // Without the join too (fix round 2): the trunk is written water of another river.
        let k = c.k(80, 31);
        c.flow[k] = 32;
        assert_eq!(fails(&c.tile(), "thalweg_beside"), 0);
    }

    /// The export's contract: labelled cells stand strictly above the ground with a surface,
    /// surfaces carry a label, and lake and sea cells sit at their bodies.json level.
    #[test]
    fn the_export_contract_checks_fire_on_bad_cells() {
        let mut c = Case::new(20, 10, |x, _| 10.0 - 0.1 * x as f32);
        let path: Vec<(u64, u64)> = (2..12).map(|x| (x, 5)).collect();
        // `river` wets its cells at the ground.
        c.river(&path, 0.1, 5);
        assert_eq!(fails(&c.tile(), "wet_not_above_ground"), 9);
        for &(x, y) in &path[..9] {
            let k = c.k(x, y);
            c.surf[k] = c.z[k] + 0.01;
        }
        let t = c.tile();
        for name in [
            "wet_not_above_ground",
            "wet_without_surface",
            "surface_without_body",
            "lake_level_json",
            "wet_unknown_body",
        ] {
            assert_eq!(fails(&t, name), 0, "{name}");
        }
        let k = c.k(3, 5);
        c.surf[k] = f32::NAN;
        let k = c.k(15, 2);
        c.surf[k] = 20.0;
        let t = c.tile();
        assert_eq!(fails(&t, "wet_without_surface"), 1);
        assert_eq!(fails(&t, "surface_without_body"), 1);
        // A lake (body 9) at 9.5 m in bodies.json: a raster surface off it fails, and so does
        // a cell whose ground reaches it (the export compares the bodies.json level).
        c.levels.insert(9, 9.5);
        let (a, b) = (c.k(1, 1), c.k(8, 1));
        (c.surf[a], c.body[a]) = (9.5001, 9);
        (c.surf[b], c.body[b]) = (9.5, 9);
        let t = c.tile();
        assert_eq!(fails(&t, "lake_level_json"), 1);
        assert_eq!(fails(&t, "wet_not_above_ground"), 1);
        assert_eq!(t.examples["wet_not_above_ground"], vec![[1, 1]]);
        // A labelled cell whose body bodies.json leaves out (the export refuses it).
        assert_eq!(fails(&t, "wet_unknown_body"), 0);
        c.unlisted.insert(5);
        assert_eq!(
            fails(&c.tile(), "wet_unknown_body"),
            9,
            "the river's 9 cells"
        );
    }

    /// An unlabelled m3 thalweg cell on a short detour 1.6 m above its river (rejoining it 4
    /// cells on) is judged as that river: its own water beside it is no second thalweg.
    #[test]
    fn a_detour_cell_is_judged_as_the_river_it_rejoins() {
        let mut c = gentle_valley(|_, v| v);
        for x in 60..65 {
            let (k, floor) = (c.k(x, 28), 20.0 - 0.01 * x as f32);
            (c.raw[k], c.z[k]) = (floor + 1.6, floor + 1.6);
        }
        let mut detour: Vec<(u64, u64)> = (60..65).map(|x| (x, 28)).collect();
        detour.extend([(65, 29), (66, 30)]);
        c.route(&detour, 16, 2.0);
        let t = c.tile();
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
    }

    /// Ground 2 m below the river 24–28 m across a gentle valley floor: dry, it is no second
    /// thalweg (ruling); with a channel (Q 0.06 km²) along it, it is.
    #[test]
    fn lower_ground_across_is_beside_only_with_a_channel() {
        let lower = |channel: bool| {
            let mut c = Case::new(130, 70, |x, y| {
                let floor = 20.0 - 0.01 * x as f32;
                floor
                    + match y {
                        54..=58 => -2.0,
                        _ => 0.02 * (y as f32 - 30.0).abs(),
                    }
            });
            let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
            c.river(&path, 2.0, 5);
            c.m3_only();
            if channel {
                let line: Vec<(u64, u64)> = (10..120).map(|x| (x, 56)).collect();
                c.route(&line, 16, 0.06);
            }
            fails(&c.tile(), "thalweg_beside")
        };
        assert_eq!(lower(false), 0);
        assert!(lower(true) > 0);
    }

    /// A hollow 1.5 m below the river, 6–8 m beside it, every cell routed straight back into
    /// it within 8 m with river-sized flow: as no thalweg (not a channel that joins) it is
    /// beside; as a thalweg joining the river it is a tributary (direction 1).
    #[test]
    fn only_a_channel_joining_the_river_is_its_tributary() {
        let hollow = |channel: bool| {
            let mut c = gentle_valley(|_, v| v);
            for x in 50..70 {
                for y in 36..39 {
                    let (k, floor) = (c.k(x, y), 20.0 - 0.01 * x as f32);
                    (c.raw[k], c.z[k]) = (floor - 1.5, floor - 1.5);
                }
                let back: Vec<(u64, u64)> = (30..39).rev().map(|y| (x, y)).collect();
                // River-sized flow either way; only a thalweg is a channel that joins.
                c.route(&back, if channel { 16 } else { 0 }, 0.06);
            }
            c.tile()
        };
        let dry = hollow(false);
        assert!(
            fails(&dry, "thalweg_beside") > 0,
            "a dry hollow draining back"
        );
        assert_eq!(dry.tributary_excused, [0, 0]);
        let trib = hollow(true);
        assert_eq!(fails(&trib, "thalweg_beside"), 0, "{:?}", trib.examples);
        assert!(trib.tributary_excused[0] > 0);
    }

    /// A dry hollow 1.5 m below the river, 6–8 m beside it (rows 36–38), its nearest row the
    /// lowest cell across; row 38 carries the channel (Q 0.06 km²) that `make` lays.
    fn hollow_with(make: impl Fn(&mut Case)) -> Tile {
        let mut c = gentle_valley(|_, v| v);
        for x in 50..70 {
            for y in 36..39 {
                let (k, floor) = (c.k(x, y), 20.0 - 0.01 * x as f32);
                (c.raw[k], c.z[k]) = (floor - 1.5, floor - 1.5);
            }
        }
        make(&mut c);
        c.tile()
    }

    /// Gate 1 ruling (4a): the channel cell beside the lowest cell is the river's own when it
    /// is a thalweg joining the river within 150 m.
    #[test]
    fn a_channel_joining_the_river_beside_the_lowest_cell_is_its_own() {
        let thalweg = |join: bool| {
            hollow_with(move |c| {
                let mut line: Vec<(u64, u64)> = (50..=70).map(|x| (x, 38)).collect();
                if join {
                    line.extend((30..38).rev().map(|y| (70, y)));
                }
                c.route(&line, 16, 0.06);
            })
        };
        let t = thalweg(false);
        assert!(fails(&t, "thalweg_beside") > 0, "no join");
        let t = thalweg(true);
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
    }

    /// Fix round 2: beside the lowest cell, a lower written river of another body (8) that
    /// never joins is not a second channel; an unwritten m3 channel there is.
    #[test]
    fn only_an_unwritten_channel_beside_the_lowest_cell_counts() {
        let water = |written: bool| {
            hollow_with(move |c| {
                let line: Vec<(u64, u64)> = (50..=70).map(|x| (x, 38)).collect();
                if written {
                    c.river(&line, 0.06, 8);
                } else {
                    c.route(&line, 16, 0.06);
                }
            })
        };
        let t = water(true);
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
        let t = water(false);
        assert!(fails(&t, "thalweg_beside") > 0, "an unwritten m3 channel");
    }

    /// A dry m2 thalweg whose carved bed rises to its receiver where the ground falls fails
    /// `bed_rises`; under standing water (an m2 node) or where the ground rises it does not.
    #[test]
    fn a_carved_bed_rising_on_a_dry_thalweg_fails() {
        let rise = |node: u32, ground_rises: bool| {
            let mut c = Case::new(9, 9, plane);
            c.river(&[(2, 4), (3, 4), (4, 4), (5, 4)], 2.0, 5);
            let (j, r) = (c.k(3, 4), c.k(4, 4));
            c.z[r] = c.z[j] + 0.05;
            c.n2[r] = node;
            if ground_rises {
                c.bz[r] = c.bz[j] + 0.1;
            }
            let t = c.tile();
            (fails(&t, "bed_rises"), t.examples.get("bed_rises").cloned())
        };
        assert_eq!(rise(0, false), (1, Some(vec![[3, 4]])));
        assert_eq!(rise(2, false).0, 0, "under standing water");
        assert_eq!(rise(0, true).0, 0, "the ground rises");
    }

    /// A river through a 2.5 m deep pool wetted as the river (R7/R26): the pool floor is its
    /// own body, so the deep water is no failure.
    #[test]
    fn a_deep_wetted_pool_passes() {
        let mut c = Case::new(130, 70, |x, y| {
            20.0 - 0.01 * x as f32 + 0.2 * (y as f32 - 30.0).abs()
        });
        let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
        c.river(&path, 2.0, 5);
        for y in 20..41 {
            for x in 40..80 {
                let k = c.k(x, y);
                let level = 20.0 - 0.01 * 40.0 + 2.5;
                if c.z[k] < level {
                    (c.surf[k], c.body[k], c.level[k]) = (level, 5, level);
                }
            }
        }
        let t = c.tile();
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
    }

    /// A steep straight reach (0.2 per cell) with a side hollow 10 m across, 1.5 m below the
    /// river there but above the river 11 m downstream: judged at the reach's grade, fine.
    #[test]
    fn a_side_hollow_on_a_steep_reach_passes() {
        let ground = |x: u64, y: u64| 50.0 - 0.2 * x as f32 + 0.05 * (y as f32 - 30.0).abs();
        let mut c = Case::new(130, 70, ground);
        let path: Vec<(u64, u64)> = (10..120).map(|x| (x, 30)).collect();
        c.river(&path, 2.0, 5);
        let k = c.k(60, 40);
        c.raw[k] = ground(60, 30) - 1.5;
        let t = c.tile();
        assert_eq!(fails(&t, "thalweg_beside"), 0, "{:?}", t.examples);
    }

    #[test]
    fn a_cut_steeper_than_raw_past_40_degrees_is_a_wall() {
        // A 0.5 slope (26.6°); lowering the cell west of (5, 5) by 2 m steepens (5, 5).
        let mut c = Case::new(11, 11, |x, _| 10.0 - 0.5 * x as f32);
        let k = c.k(6, 5);
        c.z[k] -= 2.0;
        // (5, 5) itself changed by 2 cm, so it is checked.
        let j = c.k(5, 5);
        c.z[j] -= 0.02;
        let t = c.tile();
        assert!(t.examples["walls"].contains(&[5, 5]), "{:?}", t.examples);
        let mut c = Case::new(11, 11, |x, _| 10.0 - 0.5 * x as f32);
        let j = c.k(5, 5);
        c.z[j] -= 0.02;
        assert_eq!(fails(&c.tile(), "walls"), 0, "a 2 cm cut on a gentle slope");
    }

    #[test]
    fn a_change_past_the_cap_fails() {
        let mut c = Case::new(9, 9, plane);
        let k = c.k(4, 4);
        c.z[k] -= 2.4;
        assert_eq!(fails(&c.tile(), "change_max"), 0, "within 2 m + 0.5 m");
        c.z[k] -= 0.2;
        assert_eq!(fails(&c.tile(), "change_max"), 1);
    }

    /// Written thalwegs along row 5; the final routing's along row 6 agrees (within a cell),
    /// along row 8 it does not.
    #[test]
    fn label_agreement_is_a_tolerant_iou() {
        let agree = |row: u64| {
            let mut c = Case::new(60, 12, plane);
            let written: Vec<(u64, u64)> = (2..58).map(|x| (x, 5)).collect();
            c.river(&written, 2.0, 5);
            let k: Vec<usize> = (2..57).map(|x| c.k(x, 5)).collect();
            for &k in &k {
                c.flow[k] = 32;
            }
            let fin: Vec<(u64, u64)> = (2..58).map(|x| (x, row)).collect();
            c.route(&fin, 16, 2.0);
            c.report()
        };
        assert_eq!(agree(6)["checks"]["label_agreement"], 0);
        let r = agree(8);
        assert_eq!(r["checks"]["label_agreement"], 1);
        assert_eq!(r["figures"]["label_agreement"], 0.0);
    }

    #[test]
    fn the_synthetic_world_passes_its_checks() {
        use crate::world::water::{FinishOptions, Options, finish, run, testing};
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), World { w: 64, h: 64 });
        let o = Options {
            grid: grid.clone(),
            work: dir.path().join("work"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
        };
        run(&o).expect("run");
        // Label runs before the export ruling rounded lake levels to 0.01 m; finish restores
        // the exact levels.
        let read_json = |p: &std::path::Path| -> Value {
            serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
        };
        let exact = read_json(&o.work.join("bodies.json"));
        let mut rounded = exact.clone();
        let mut lakes = 0;
        for b in rounded["bodies"].as_array_mut().unwrap() {
            if b["kind"] == "lake" {
                let lv = b["level_m"].as_f64().unwrap();
                b["level_m"] = json!((lv * 100.0).round() / 100.0);
                lakes += 1;
            }
        }
        assert!(lakes > 0);
        std::fs::write(
            o.work.join("bodies.json"),
            serde_json::to_vec(&rounded).unwrap(),
        )
        .unwrap();
        let f = FinishOptions {
            grid: grid.clone(),
            work: o.work.clone(),
            ground: o.work.join("carved.f32"),
            out: dir.path().join("out"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
            spots: Vec::new(),
        };
        let outcome = finish(&f).expect("finish");
        let report = read_json(&dir.path().join("out/report.json"));
        assert!(
            outcome.passed,
            "{}",
            serde_json::to_string_pretty(&report).unwrap()
        );
        assert_eq!(read_json(&dir.path().join("out/bodies.json")), exact);
        for name in [
            "wet_not_above_ground",
            "wet_without_surface",
            "surface_without_body",
            "lake_level_json",
            "wet_unknown_body",
        ] {
            assert_eq!(report["checks"][name], 0, "{name}");
        }
        for name in [
            "elevation.f32",
            "water-surface.f32",
            "water-body.u32",
            "discharge.f32",
            "flow.u8",
            "bodies.json",
            "grid.json",
        ] {
            assert!(dir.path().join("out").join(name).exists(), "{name}");
        }
        assert!(finish(&f).is_err(), "an existing output is refused");
    }

    #[test]
    fn a_raised_rim_below_a_lake_fails_the_dam_check() {
        use crate::world::water::{FinishOptions, Options, finish, run, testing};
        let dir = tempfile::tempdir().expect("temp");
        let grid = testing::grid_dir(dir.path(), World { w: 64, h: 64 });
        let o = Options {
            grid: grid.clone(),
            work: dir.path().join("work"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
        };
        run(&o).expect("run");
        // Raise the dry cells beside the humid lake's exit by 2 m in a copy of the ground.
        let bodies: Value =
            serde_json::from_slice(&std::fs::read(o.work.join("bodies.json")).unwrap()).unwrap();
        let lake = bodies["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["kind"] == "lake" && !b["exit_cell"].is_null())
            .expect("a full lake");
        let (ex, ey) = (
            lake["exit_cell"][0].as_u64().unwrap(),
            lake["exit_cell"][1].as_u64().unwrap(),
        );
        let read = |p: &std::path::Path| -> Vec<f32> {
            std::fs::read(p)
                .unwrap()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect()
        };
        let mut z = read(&o.work.join("carved.f32"));
        let surface = read(&o.work.join("water-surface.f32"));
        for y in ey.saturating_sub(2)..=(ey + 2).min(63) {
            for x in ex.saturating_sub(2)..=(ex + 2).min(63) {
                let k = (y * 64 + x) as usize;
                if surface[k].is_nan() {
                    z[k] = lake["level_m"].as_f64().unwrap() as f32 + 2.0;
                }
            }
        }
        let bumped = dir.path().join("bumped.f32");
        std::fs::write(
            &bumped,
            z.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
        )
        .unwrap();
        let f = FinishOptions {
            grid: grid.clone(),
            work: o.work.clone(),
            ground: bumped,
            out: dir.path().join("out"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
            spots: Vec::new(),
        };
        let outcome = finish(&f).expect("finish");
        assert!(!outcome.passed);
        let report: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("out/report.json")).unwrap())
                .unwrap();
        assert!(
            report["checks"]["dam_rims"].as_u64().unwrap() > 0,
            "{}",
            report["checks"]
        );
    }
}
