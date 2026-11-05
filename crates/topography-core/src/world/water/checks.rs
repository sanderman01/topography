//! Hard checks (each a count that must be 0; spec "Checks", rulings R7, R11, R14, R16, R22,
//! R27, R29, R30, R31) and figures on a finished run, tile by tile with a 160-cell halo (a
//! 150 m straight run at 1 m must fit).

use super::bodies::main_up;
use super::geom::{OFFSETS, Rect, UnionFind, step_len};
use super::layers::Layer;
use super::pipeline::Setup;
use super::{BODY_BUDGET, MAIN_RIVER_KM2};
use crate::acquisition::Result;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const HALO: u64 = 160;
/// Parallel rivers (R29): another flow line from this size, this close, this parallel, this long.
const PARALLEL_KM2: f32 = 0.002;
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
const SILL_M: f32 = 0.01;
const DAM_STEP_M: f32 = 1.5;
const DAM_RIM_M: f32 = 0.5;
/// Exit falls are read this far below each exit (figures; the 50 m one is also `dam_steps`'s
/// reach).
const FALL_M: [f64; 3] = [5.0, 10.0, 50.0];
const ACROSS_M: f64 = 30.0;
const THALWEG_M: f32 = 1.0;
const STRAIGHT_M: f64 = 60.0;
const STRAIGHT_SHARE: f64 = 0.05;
const PLANE_RMS_M: f64 = 0.02;
const PLANE_DEG: f64 = 5.0;
const WALL_DEG: f64 = 40.0;
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
}

/// Lakes by id: level, full, outlet [inside, outside], cells needed (closed).
pub(crate) type LakeInfo = BTreeMap<u32, (f32, bool, [u64; 2], u64)>;

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
    /// Straight-run clusters over 60 m whose longest member starts in this tile.
    pub runs: Vec<Run>,
    /// Failing straight runs that lie mostly in ponds (reported apart, still failing).
    pub pond_runs: u64,
    /// Raw-independent dam figures (R31): (node key, lake, cell, drop m).
    pub drops_any: Vec<(u64, Option<u32>, [u64; 2], f32)>,
    /// Dry cells beside an exit above its level: (node key, lake, cell, after m, before m).
    pub rims_any: Vec<Rim>,
    /// Label agreement (R27): written thalweg cells, of them matched within one cell, m3
    /// thalweg cells, of them matched.
    pub agree: [u64; 4],
    /// Rivers with Q ≥ 1 km² ending in a closed node (M2).
    pub closed_ends: u64,
    /// Bank water over 0.5 m above the dry cell it drains into: count, highest, examples.
    pub bank_steps: (u64, f32, Vec<[u64; 2]>),
    /// Cells per class, and (class, lake) pairs seen.
    pub class_cells: BTreeMap<u8, u64>,
    pub class_lakes: BTreeSet<(u8, u32)>,
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

/// A changed cell's slope (Horn, degrees) fails `walls` against its raw slope (shared with
/// the channels stage, which shrinks cuts until none fails).
pub(crate) fn is_wall(slope: f64, raw_slope: f64) -> bool {
    slope > WALL_DEG.max(raw_slope) + 0.01
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
    exits: &[Exit],
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

    // The lowest D over cells of window cell j's body within `r` cells of it (D(j) included):
    // downstream own cells within reach count, so a steep reach is judged at its own grade.
    let reach_across = (ACROSS_M / s.spacing_m).round() as i64;
    let own_min = |j: usize, r: f64| -> f32 {
        let (x, y) = win.world_xy(j);
        let reach = r.ceil() as i64;
        let mut low = dsurf(j);
        for oy in -reach..=reach {
            for ox in -reach..=reach {
                if ((ox * ox + oy * oy) as f64).sqrt() > r {
                    continue;
                }
                if let Some(c) = at_i(x as i64 + ox, y as i64 + oy)
                    && body[c] == body[j]
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
        let failing = members.iter().find(|&&r| {
            let mr = &maximal[r];
            mr.1 > STRAIGHT_RUN_M && raw_share(mr) < GORGE_SHARE
        });
        if let Some(&r) = failing {
            t.fail("straight_runs", x, y);
            if pond_share(&maximal[r]) > 0.5 {
                t.pond_runs += 1;
            }
        }
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
        if node[j] >= 2 && !lake_cell(j) {
            t.pond_cells += 1;
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
        // Lakes are flat, stand at their lowest saddle and do not leak (C1). A lake cell
        // whose ground reaches its level holds no water: it is shore on both sides, never
        // a leak's source, and a neighbour at the level (D = level).
        if let Some(&(lv, full, outlet, _)) = lake_cell(j).then(|| &lakes[&body[j]]) {
            t.class_lakes.insert((class, body[j]));
            if surf[j] != lv {
                t.fail("lake_not_flat", x, y);
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
        // (inside the world the 160-cell halo always holds them).
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
            if is_wall(fs, rs) {
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
            // One thalweg (I3, spec): across the flow, out to the first divide on each side (raw
            // more than 1 m above D here), the lowest raw cell is a cell of this river's body
            // (wetted river-crossed ponds included) or lies no more than 1 m below the lowest
            // D of this body within its distance + 1 cell. Too low a river is no failure:
            // channels cut 2 m at most, and `walls` / `change_max` cover over-cutting.
            if f64::from(q[j]) >= MAIN_RIVER_KM2
                && let Some((vx, vy)) = direction(j)
            {
                let (nx, ny) = (-vy, vx);
                let dj = dsurf(j);
                let mut low = (raw[j], 0i64, j);
                for side in [-1i64, 1] {
                    for i in 1..=reach_across {
                        let Some(p) = at_i(
                            (x as f64 + nx * (side * i) as f64).round() as i64,
                            (y as f64 + ny * (side * i) as f64).round() as i64,
                        ) else {
                            break;
                        };
                        if raw[p] > dj + THALWEG_M {
                            break;
                        }
                        if raw[p] < low.0 || (raw[p] == low.0 && i < low.1) {
                            low = (raw[p], i, p);
                        }
                    }
                }
                let (zl, _, p) = low;
                let own = body[j] > 1 && body[p] == body[j];
                if !own {
                    let (px, py) = win.world_xy(p);
                    let r = (px as f64 - x as f64).hypot(py as f64 - y as f64);
                    // An unlabelled thalweg (body ≤ 1) is judged against itself only.
                    let d = if body[j] > 1 { own_min(j, r + 1.0) } else { dj };
                    if zl < d - THALWEG_M {
                        t.fail("thalweg_beside", x, y);
                    }
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
    Ok(t)
}

pub(crate) struct CheckExtra {
    /// Cells on flow cycles in the third model run (the model refuses cycles, so 0 here).
    pub cycles: u64,
    pub bodies: usize,
    /// Closed lakes covering fewer cells than their supply needs.
    pub lake_short: u64,
    pub relax: Option<Value>,
    /// The lakes, for the lowest-saddle check over all tiles.
    pub lakes: LakeInfo,
    pub cell_km2: f64,
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
        "lake_not_flat",
        "lake_leaks",
        "lake_exit_surface",
        "dam_steps",
        "dam_rims",
        "thalweg_beside",
        "parallel_rivers",
        "straight_runs",
        "walls",
        "change_max",
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
    let (mut agree, mut closed_ends, mut pond_runs) = ([0u64; 4], 0u64, 0u64);
    let mut bank = (0u64, 0f32, Vec::new());
    let mut class_cells: BTreeMap<u8, u64> = BTreeMap::new();
    let mut class_lakes: BTreeSet<(u8, u32)> = BTreeSet::new();
    for t in tiles {
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
        pond_runs += t.pond_runs;
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
    let overlap = extra
        .relax
        .as_ref()
        .and_then(|r| r["max_overlap_diff_m"].as_f64());
    fails.insert(
        "relax_overlap",
        u64::from(overlap.is_some_and(|d| d > 0.001)),
    );
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
    figures.insert("straight_runs_in_ponds".into(), json!(pond_runs));
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
    figures.insert("big_rivers_into_closed_nodes".into(), json!(closed_ends));
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
    figures.insert("relax".into(), extra.relax.unwrap_or(Value::Null));
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
        lakes: LakeInfo,
        exits: Vec<Exit>,
    }

    impl Case {
        fn new(w: u64, h: u64, ground: impl Fn(u64, u64) -> f32) -> Self {
            let world = World { w, h };
            let n = (w * h) as usize;
            let z: Vec<f32> = (0..n as u64).map(|i| ground(i % w, i / w)).collect();
            Case {
                world,
                raw: z.clone(),
                z,
                surf: vec![f32::NAN; n],
                body: vec![0; n],
                level: vec![f32::NAN; n],
                node: vec![0; n],
                flow: vec![32; n],
                q: vec![0.0; n],
                fw: vec![0; n],
                lakes: LakeInfo::new(),
                exits: Vec::new(),
            }
        }
        fn k(&self, x: u64, y: u64) -> usize {
            (y * self.world.w + x) as usize
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
        fn tile(&self) -> Tile {
            let w = self.world;
            let l = CheckLayers {
                raw: Layer::memory(w, self.raw.clone()),
                ground: Layer::memory(w, self.z.clone()),
                surface: Layer::memory(w, self.surf.clone()),
                body: Layer::memory(w, self.body.clone()),
                level: Layer::memory(w, self.level.clone()),
                node: Layer::memory(w, self.node.clone()),
                flow: Layer::memory(w, self.flow.clone()),
                discharge: Layer::memory(w, self.q.clone()),
                flow_written: Layer::memory(w, self.fw.clone()),
            };
            check_tile(
                &setup(w, 4096, 0.05),
                &l,
                Rect::whole(w),
                &self.lakes,
                &self.exits,
            )
            .expect("check")
        }
        fn report(&self) -> Value {
            let extra = CheckExtra {
                cycles: 0,
                bodies: 0,
                lake_short: 0,
                relax: None,
                lakes: self.lakes.clone(),
                cell_km2: 1e-6,
            };
            report(vec![self.tile()], extra, Map::new()).0
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
    /// plane), and optionally a second flow line along row `beside` (x 10..240, gaps every
    /// `gap` cells) that never joins it.
    fn river_on(
        w: u64,
        h: u64,
        path: &[(u64, u64)],
        beside: Option<(u64, u64)>,
        raw: Option<Vec<f32>>,
    ) -> Tile {
        let mut c = Case::new(w, h, plane);
        c.river(path, 2.0, 5);
        if let Some((row, gap)) = beside {
            for x in 10..240 {
                let k = c.k(x, row);
                (c.flow[k], c.q[k]) = (if gap > 0 && x % gap == 0 { 0 } else { 4 }, 0.01);
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
        c.route(&line, 0, 0.01);
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
        assert_eq!(fails(&t, "straight_runs"), 1, "one 1:10 run of 228 m");
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
        let t = river_on(260, 30, &wobble(0.5), None, None);
        assert_eq!(fails(&t, "straight_runs"), 1, "{:?}", t.runs.first());
        let t = river_on(260, 30, &wobble(1.5), None, None);
        assert_eq!(fails(&t, "straight_runs"), 0, "{:?}", t.runs.first());
    }

    /// A long, gentle arc: many maximal runs overlap along it; they are one straight reach.
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
        assert_eq!(fails(&t, "straight_runs"), 1, "{:?}", t.runs);
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
        assert_eq!(
            fails(&c.tile(), "thalweg_beside"),
            0,
            "the ridge is a divide"
        );
        // Its own wet bank cell lowest across: fine.
        let mut c = Case::new(130, 70, ground);
        c.river(&path, 2.0, 5);
        for &(x, _) in &path {
            let k = c.k(x, 31);
            c.raw[k] -= 0.5;
            (c.surf[k], c.body[k]) = (c.z[c.k(x, 30)], 5);
        }
        assert_eq!(fails(&c.tile(), "thalweg_beside"), 0);
        // A river incised 1.5 m below every raw cell across is no failure (one-sided).
        let mut c = Case::new(130, 70, ground);
        c.river(&path, 2.0, 5);
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
    #[test]
    fn a_lower_parallel_channel_is_beside() {
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
        let t = c.tile();
        assert!(fails(&t, "thalweg_beside") > 0, "{:?}", t.examples);
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
        let f = FinishOptions {
            grid: grid.clone(),
            work: o.work.clone(),
            ground: o.work.join("carved.f32"),
            out: dir.path().join("out"),
            tile: 16,
            threads: 2,
            water: Some(grid.join("water.toml")),
            river_km2: Some(1e-5),
            relax: None,
        };
        let outcome = finish(&f).expect("finish");
        let report: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("out/report.json")).unwrap())
                .unwrap();
        assert!(
            outcome.passed,
            "{}",
            serde_json::to_string_pretty(&report).unwrap()
        );
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
            relax: None,
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
