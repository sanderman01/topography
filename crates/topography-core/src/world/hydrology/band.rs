//! Bands: a grid whose top rows are the previous (northern) band's core (`--north`) takes
//! those rows as they are, so adjacent bands agree on the seam by construction, and the seam
//! check that compares two band outputs.

use super::flood::{CENTRE, Dims, FLOW_CENTRE, FLOW_DRY, RIVER, SEA, receiver};
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

/// Judgment constant: rows below the seam over which this band's input ground blends from the
/// previous band's ground (continuous with the rows taken from it) into its own.
pub(super) const SEAM_BLEND_ROWS: usize = 64;

/// The northern band's outputs over this grid's top `rows` rows (its core).
pub(super) struct North {
    pub rows: usize,
    pub ground: Vec<f32>,
    /// Its ground over the first rows after `rows` (up to `SEAM_BLEND_ROWS`), whole rows.
    pub halo_ground: Vec<f32>,
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

/// Body kinds by id, and the highest id the band used (`next_id` - 1 when given: a band
/// whose newest bodies were dropped still reserves their ids).
fn kinds(bodies: &Value) -> (HashMap<u32, BodyKind>, u32) {
    let mut kinds = HashMap::new();
    let mut max_id = (bodies["next_id"].as_u64().unwrap_or(2) as u32)
        .saturating_sub(1)
        .max(1);
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
    let bodies_bytes = grid::read_bodies(&dir.join("bodies.json"))?;
    let (kinds, max_id) = kinds(&serde_json::from_slice(&bodies_bytes)?);
    let mut digests = Map::new();
    digests.insert("grid.json".into(), digest(&bytes).into());
    digests.insert("bodies.json".into(), digest(&bodies_bytes).into());
    // Its ground over this grid's first free rows (its halo), as far as it reaches.
    let halo = [t + rows, (t + rows + SEAM_BLEND_ROWS).min(a.h)];
    let halo_ground = grid::read_window(&dir.join("elevation.f32"), size, xs, halo, f32le)?;
    Ok(North {
        rows,
        halo_ground,
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

/// Blend this grid's first free rows from the previous band's ground (`halo`, whole rows
/// after `fixed_rows`) into its own over `SEAM_BLEND_ROWS`: that ground is continuous with the
/// rows taken from it, so the seam starts without a step whichever band carved there. Where
/// that band carved its halo below the seam limit (`limit`, whole rows from `seam_limit`; a
/// channel its hydrology guessed there), the blend takes no more of the cut than the limit
/// allows, nor below this band's own ground.
pub(super) fn blend_seam(
    dims: Dims,
    fixed_rows: usize,
    ground: &mut [f32],
    halo: &[f32],
    limit: &[f32],
) {
    let w = dims.w;
    let rows = (halo.len() / w).min(dims.h.saturating_sub(fixed_rows));
    for d in 0..rows {
        let k = 1.0 - d as f32 / SEAM_BLEND_ROWS as f32;
        let row = (fixed_rows + d) * w;
        for x in 0..w {
            let (h, g) = (halo[d * w + x], ground[row + x]);
            let f = limit
                .get(d * w + x)
                .map_or(f32::NEG_INFINITY, |&f| f.min(g));
            if h.is_finite() {
                ground[row + x] = k * h.max(f) + (1.0 - k) * g;
            }
        }
    }
}

/// The seam limit over the free rows after `fixed_rows` (whole rows, as far as a carve
/// `depth_m` deep can matter): the previous band's dry ground in its last row less the side
/// slope (`tan`) over the distance, or beside water there (`wet`) that water's bed plus the
/// slope, where lower; right beside a lake or the sea there (`still`, its level) no higher than
/// that level (a shore at the level is no step).
#[allow(clippy::too_many_arguments)]
pub(super) fn seam_limit(
    dims: Dims,
    fixed_rows: usize,
    ground: &[f32],
    wet: impl Fn(usize) -> bool,
    still: impl Fn(usize) -> Option<f32>,
    depth_m: f32,
    tan: f32,
    spacing_m: f64,
) -> Vec<f32> {
    let w = dims.w;
    if fixed_rows == 0 {
        return Vec::new();
    }
    let last = (fixed_rows - 1) * w;
    // Dry ground sets the limit; beside water in that row it may go down to the water's bed
    // and up from it at the side slope (a stream reaching the water there is not dammed).
    let source: Vec<f32> = (0..w)
        .map(|x| {
            if wet(last + x) {
                f32::NEG_INFINITY
            } else {
                ground[last + x]
            }
        })
        .collect();
    let water: Vec<f32> = (0..w)
        .map(|x| {
            if wet(last + x) {
                ground[last + x]
            } else {
                f32::INFINITY
            }
        })
        .collect();
    // Beyond this many cells a source's limit lies below every carve's depth bound.
    let reach = (depth_m / (tan * spacing_m as f32)).ceil() as usize + 1;
    let rows = reach.min(dims.h - fixed_rows);
    let mut out = vec![f32::NEG_INFINITY; rows * w];
    for dy in 1..=rows {
        let drop: Vec<f32> = (0..=reach)
            .map(|dx| tan * ((dx * dx + dy * dy) as f32).sqrt() * spacing_m as f32)
            .collect();
        for x in 0..w {
            let (a, b) = (x.saturating_sub(reach), (x + reach).min(w - 1));
            let dry = (a..=b)
                .map(|s| source[s] - drop[s.abs_diff(x)])
                .fold(f32::NEG_INFINITY, f32::max);
            let wet = (a..=b)
                .map(|s| water[s] + drop[s.abs_diff(x)])
                .fold(f32::INFINITY, f32::min);
            out[(dy - 1) * w + x] = dry.min(wet);
        }
    }
    for x in 0..rows.min(1) * w {
        let (a, b) = (x.saturating_sub(1), (x + 1).min(w - 1));
        for s in a..=b {
            if let Some(level) = still(last + s) {
                out[x] = out[x].min(level);
            }
        }
    }
    out
}

/// Lowest ground each free cell may be carved to: at most `depth_m` below its ground now, and
/// near the seam not below the previous band's dry ground in its last row less the side slope
/// (`tan`) over the distance, so no cut leaves a step at the seam. Beside water there (`wet`)
/// the limit is that water's bed plus the side slope instead, where lower: rivers and lakes
/// cross the seam at their own beds. Fixed cells get +inf (never carved). Also the seam limit
/// alone over the rows it reaches after `fixed_rows` (whole rows; coarse river paths keep only
/// that one, as the coarse network sets where they run).
#[allow(clippy::too_many_arguments)]
pub(super) fn ground_floor(
    dims: Dims,
    fixed_rows: usize,
    ground: &[f32],
    wet: impl Fn(usize) -> bool,
    still: impl Fn(usize) -> Option<f32>,
    depth_m: f32,
    tan: f32,
    spacing_m: f64,
) -> (Vec<f32>, Vec<f32>) {
    let w = dims.w;
    let mut floor: Vec<f32> = ground.iter().map(|&g| g - depth_m).collect();
    floor[..fixed_rows * w].fill(f32::INFINITY);
    let mut seam_rows = seam_limit(
        dims, fixed_rows, ground, wet, still, depth_m, tan, spacing_m,
    );
    for (k, seam) in seam_rows.iter_mut().enumerate() {
        let i = fixed_rows * w + k;
        // Never above the cell's own ground (natural slopes steeper than `tan` stay).
        *seam = seam.min(ground[i]);
        floor[i] = floor[i].max(*seam);
    }
    (floor, seam_rows)
}

/// No free cell (`reentry`).
pub(super) const NO_ENTRY: u32 = u32::MAX;

/// Per fixed cell (the first `fixed_rows` rows): the free cell of this grid its flow in the
/// previous band (`flow` there) comes back into, or `NO_ENTRY`.
pub(super) fn reentry(dims: Dims, flow: &[u8], fixed_rows: usize) -> Vec<u32> {
    const UNKNOWN: u32 = u32::MAX - 1;
    const OPEN: u32 = u32::MAX - 2;
    let m = (fixed_rows * dims.w).min(flow.len());
    let mut state = vec![UNKNOWN; m];
    let mut chain = Vec::new();
    for start in 0..m {
        if state[start] != UNKNOWN {
            continue;
        }
        let mut c = start;
        let entry = loop {
            if c >= m {
                break c as u32;
            }
            match state[c] {
                UNKNOWN => {}
                OPEN => break NO_ENTRY,
                s => break s,
            }
            state[c] = OPEN;
            chain.push(c);
            let d = flow[c] & 15;
            match (d != 0)
                .then(|| dims.neighbour(c, d as usize - 1))
                .flatten()
            {
                Some(r) => c = r,
                None => break NO_ENTRY,
            }
        };
        for x in chain.drain(..) {
            state[x] = entry;
        }
    }
    state
}

/// Streams (discharge from `river_km2`) in this band's first row that drain back across the
/// seam onto the previous band's dry ground, where that band's river runs a few cells over
/// (its halo guess put the stream there): route them along the first rows into the river, on
/// ground lowered to a running minimum so the route never climbs. A river cell above the
/// stream's own ground is not joined. Returns the streams joined.
#[allow(clippy::too_many_arguments)]
pub(super) fn join_seam_rivers(
    dims: Dims,
    fixed_rows: usize,
    ground: &mut [f32],
    flags: &[u8],
    body: &[u32],
    reenters: &[bool],
    dir: &mut [u8],
    acc: &[f32],
    river_km2: f32,
    carve_floor: &[f32],
) -> u64 {
    // Rows the route may use, and its steps (halo guesses differ by several metres).
    const REACH: usize = 3;
    const STEPS: usize = 8;
    let w = dims.w;
    let first = fixed_rows * w;
    if fixed_rows == 0 || first + w > ground.len() {
        return 0;
    }
    let taken = |j: usize| j < first;
    // A taken river cell ends the route if its flow in the previous band never comes back
    // into this band (a loop across the seam, or the stream joining itself).
    let last_row: Vec<bool> = (first - w..first)
        .map(|j| flags[j] & RIVER != 0 && !reenters.get(j).copied().unwrap_or(true))
        .collect();
    let river = |j: usize| j < first && j + w >= first && last_row[j + w - first];
    let mut joined = 0;
    for i in first..first + w {
        let onto_dry =
            receiver(dims, dir, i).is_some_and(|r| taken(r) && flags[r] & (RIVER | SEA) == 0);
        if acc[i] < river_km2 || !onto_dry {
            continue;
        }
        // Breadth first along this band's first rows to a cell beside the taken river.
        let mut from: HashMap<usize, usize> = HashMap::from([(i, i)]);
        let mut layer = vec![i];
        let mut end = None;
        'search: for _ in 0..=STEPS {
            let mut next = Vec::new();
            for &c in &layer {
                let target = dims
                    .neighbours(c)
                    .filter(|&(_, j)| river(j) && ground[j] <= ground[i])
                    .min_by(|a, b| ground[a.1].total_cmp(&ground[b.1]));
                if let Some((k, _)) = target {
                    end = Some((c, k));
                    break 'search;
                }
                for (_, j) in dims.neighbours(c) {
                    let near = j >= first && j < first + REACH * w;
                    // Not through water or basins (their exits route them).
                    let open = flags[j] & (SEA | RIVER) == 0 && body[j] == 0;
                    if near && open && !from.contains_key(&j) {
                        from.insert(j, c);
                        next.push(j);
                    }
                }
            }
            layer = next;
        }
        let Some((last, k)) = end else { continue };
        let mut route = vec![last];
        while *route.last().unwrap_or(&i) != i {
            let c = route[route.len() - 1];
            route.push(from[&c]);
        }
        route.reverse();
        let low = route
            .iter()
            .map(|&c| ground[c])
            .fold(f32::INFINITY, f32::min);
        let into = dims.neighbour(last, k).map_or(f32::INFINITY, |j| ground[j]);
        if into > low {
            continue;
        }
        // Nor a route the carve floor would keep from descending (it would need a trench).
        let mut level = ground[route[0]];
        let climbs = route.iter().skip(1).any(|&c| {
            let f = carve_floor.get(c).copied().unwrap_or(f32::NEG_INFINITY);
            level = level.min(ground[c]);
            f > level
        });
        if climbs {
            continue;
        }
        dir[last] = k as u8 + 1;
        for p in route.windows(2) {
            dir[p[0]] = dims.towards(p[0], p[1]);
            let to = carve_floor
                .get(p[1])
                .map_or(ground[p[0]], |&f| ground[p[0]].max(f));
            ground[p[1]] = ground[p[1]].min(to);
        }
        joined += 1;
    }
    joined
}

/// Rivers entering from the previous band (`inflow`: fixed centreline cell, free cell) whose
/// routing here leaves back across the seam onto that band's dry ground (its halo guess ran
/// elsewhere): reroute each from its entry over free cells to water, a basin, the grid edge or
/// the previous band's river, on ground lowered to a running minimum so the route never
/// climbs, and only over cells whose carve floor allows that (no trench). A river with no such
/// way is left in `stuck` (its entry ends in a sink).
#[allow(clippy::too_many_arguments)]
pub(super) fn reroute_inflows(
    dims: Dims,
    fixed_rows: usize,
    ground: &mut [f32],
    flags: &[u8],
    body: &[u32],
    reentry: &[u32],
    dir: &mut [u8],
    inflow: &[(usize, usize)],
    carve_floor: &[f32],
) -> Reroute {
    // Cells a search may visit before it gives up.
    const MAX_CELLS: usize = 1 << 22;
    let first = fixed_rows * dims.w;
    let water = |x: usize| flags[x] & (RIVER | SEA) != 0 || body[x] != 0;
    let fixed_end = |x: usize| water(x) && reentry[x] == NO_ENTRY;
    // Whether the routing from free cell `c` ends at water, a basin, an edge or a fixed end
    // (through fixed water that comes back here, unless it comes back onto this route).
    let ends_well = |memo: &mut HashMap<usize, bool>, dir: &[u8], c: usize| {
        let mut chain = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut x = c;
        let ok = loop {
            if let Some(&v) = memo.get(&x) {
                break v;
            }
            if x < first {
                if !water(x) {
                    break false;
                }
                match reentry[x] {
                    NO_ENTRY => break true,
                    e => x = e as usize,
                }
                continue;
            }
            if flags[x] & SEA != 0 || body[x] != 0 {
                break true;
            }
            if !seen.insert(x) {
                break false;
            }
            chain.push(x);
            match receiver(dims, dir, x) {
                Some(r) => x = r,
                None => break true,
            }
        };
        for y in chain {
            memo.insert(y, ok);
        }
        ok
    };
    let mut cut = Reroute::default();
    for &(i, r) in inflow {
        let mut memo: HashMap<usize, bool> = HashMap::new();
        if flags[i] & CENTRE == 0 || ends_well(&mut memo, dir, r) {
            continue;
        }
        // Along the way whose water level (the running minimum of the ground) stays highest,
        // only over cells that can be lowered to that level (their carve floor is not above
        // it): the route never climbs, so it needs no trench.
        let floor = |c: usize| carve_floor.get(c).copied().unwrap_or(f32::NEG_INFINITY);
        let mut from: HashMap<usize, usize> = HashMap::from([(r, r)]);
        let mut heap = std::collections::BinaryHeap::from([Widest(ground[r], ground[r], r)]);
        let mut end = None;
        'search: while let Some(Widest(level, _, c)) = heap.pop() {
            if c != r && ends_well(&mut memo, dir, c) {
                end = Some((c, None));
                break;
            }
            for (_, j) in dims.neighbours(c) {
                if from.contains_key(&j) {
                    continue;
                }
                if j < first {
                    if fixed_end(j) && ground[j] <= level {
                        end = Some((c, Some(j)));
                        break 'search;
                    }
                    continue;
                }
                if floor(j) > level {
                    continue;
                }
                from.insert(j, c);
                heap.push(Widest(level.min(ground[j]), ground[j], j));
            }
            if from.len() > MAX_CELLS {
                break;
            }
        }
        let Some((last, into)) = end else {
            // No way out that does not climb: the river ends here (a sink).
            cut.stuck.push(r);
            continue;
        };
        let mut route = vec![last];
        while route[route.len() - 1] != r {
            route.push(from[&route[route.len() - 1]]);
        }
        route.reverse();
        let mut lower = |ground: &mut [f32], c: usize, to: f32| {
            let to = carve_floor.get(c).map_or(to, |&f| to.max(f));
            if to < ground[c] {
                cut.max_m = cut.max_m.max(ground[c] - to);
                cut.cells += 1;
                ground[c] = to;
            }
        };
        for p in route.windows(2) {
            dir[p[0]] = dims.towards(p[0], p[1]);
            lower(ground, p[1], ground[p[0]]);
        }
        // On to the end: into the fixed river, or down the routing until it no longer climbs.
        let g = ground[last];
        match into {
            Some(j) => dir[last] = dims.towards(last, j),
            None => {
                let mut x = receiver(dims, dir, last);
                while let Some(y) = x.filter(|&y| y >= first && flags[y] & SEA == 0) {
                    if body[y] != 0 || ground[y] <= g {
                        break;
                    }
                    lower(ground, y, g);
                    x = receiver(dims, dir, y);
                }
            }
        }
        cut.rivers += 1;
    }
    cut
}

/// What `reroute_inflows` did: rivers rerouted, cells lowered and the deepest cut, and the
/// entry cells of rivers with no way out that does not climb.
#[derive(Clone, Default)]
pub(super) struct Reroute {
    pub rivers: u64,
    pub cells: u64,
    pub max_m: f32,
    pub stuck: Vec<usize>,
}

/// Max-heap entry: the highest water level first, then the lowest ground, then the lowest index.
pub(super) struct Widest(pub f32, pub f32, pub usize);
impl PartialEq for Widest {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o).is_eq()
    }
}
impl Eq for Widest {}
impl Ord for Widest {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0
            .total_cmp(&o.0)
            .then(o.1.total_cmp(&self.1))
            .then(o.2.cmp(&self.2))
    }
}
impl PartialOrd for Widest {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
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
        let (k, _) = kinds(&serde_json::from_slice(&grid::read_bodies(
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

    /// A 6 x 3 grid whose first row is the previous band's: its river at (3, 0). A stream at
    /// (1, 1) drains back north onto that band's dry ground at (1, 0).
    fn seam_scene(river_ground: f32) -> (Dims, Vec<f32>, Vec<u8>, Vec<u8>, Vec<f32>) {
        let dims = Dims { w: 6, h: 3 };
        let mut ground = vec![0.5f32; 18];
        (ground[3], ground[7], ground[8]) = (river_ground, 0.0, 0.3);
        let mut flags = vec![0u8; 18];
        for f in flags.iter_mut().take(6) {
            *f = super::super::flood::FIXED;
        }
        flags[3] |= RIVER | CENTRE;
        let mut dir = vec![0u8; 18];
        dir[7] = 1; // north, onto taken dry ground
        let mut acc = vec![0.0f32; 18];
        acc[7] = 1.0;
        (dims, ground, flags, dir, acc)
    }

    #[test]
    fn a_stream_turning_back_across_the_seam_is_led_into_the_river() {
        let (dims, mut ground, flags, mut dir, acc) = seam_scene(-1.0);
        assert_eq!(
            join_seam_rivers(
                dims,
                1,
                &mut ground,
                &flags,
                &[0; 18],
                &[false; 6],
                &mut dir,
                &acc,
                0.5,
                &[]
            ),
            1
        );
        // (1, 1) -> (2, 1) -> (3, 0), the route lowered so it never climbs.
        let mut c = 7;
        let mut path = vec![c];
        while let Some(r) = receiver(dims, &dir, c) {
            assert!(ground[r] <= ground[c], "{c} -> {r} climbs");
            path.push(r);
            c = r;
        }
        assert_eq!(c, 3);
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn a_river_above_the_stream_is_not_joined() {
        let (dims, mut ground, flags, mut dir, acc) = seam_scene(0.2);
        let before = dir.clone();
        assert_eq!(
            join_seam_rivers(
                dims,
                1,
                &mut ground,
                &flags,
                &[0; 18],
                &[false; 6],
                &mut dir,
                &acc,
                0.5,
                &[]
            ),
            0
        );
        assert_eq!(dir, before);
    }

    #[test]
    fn a_taken_river_whose_flow_comes_back_is_no_join_target() {
        // The taken river cell (3, 0) drains south into this band at (3, 1): joining it
        // would close a loop across the seam.
        let (dims, mut ground, flags, mut dir, acc) = seam_scene(-1.0);
        let mut flow = vec![0u8; 6];
        flow[3] = 2;
        let entry = reentry(dims, &flow, 1);
        assert_eq!((entry[3], entry[1]), (9, NO_ENTRY));
        let back: Vec<bool> = entry.iter().map(|&r| r != NO_ENTRY).collect();
        let before = dir.clone();
        let joined = join_seam_rivers(
            dims,
            1,
            &mut ground,
            &flags,
            &[0; 18],
            &back,
            &mut dir,
            &acc,
            0.5,
            &[],
        );
        assert_eq!(joined, 0);
        assert_eq!(dir, before);
    }

    #[test]
    fn a_river_turning_back_onto_taken_dry_ground_is_rerouted() {
        // 4 x 4, first row taken: the river (0, 0) enters at (0, 1), whose routing runs east
        // and back north onto taken dry ground at (3, 0). The lowest way out is south to
        // the edge at (0, 3).
        let dims = Dims { w: 4, h: 4 };
        let mut ground = vec![9.0; 16];
        for (i, z) in [(4, 5.0), (8, 6.0), (12, 4.0)] {
            ground[i] = z;
        }
        ground[5..8].fill(7.0);
        ground[9..12].fill(7.0);
        ground[13..16].fill(7.0);
        let mut flags = vec![0u8; 16];
        flags[0] = RIVER | CENTRE;
        let mut dir = vec![0u8; 16];
        (dir[4], dir[5], dir[6], dir[8]) = (4, 4, 6, 2);
        let back = [4, NO_ENTRY, NO_ENTRY, NO_ENTRY];
        let n = reroute_inflows(
            dims,
            1,
            &mut ground,
            &flags,
            &[0; 16],
            &back,
            &mut dir,
            &[(0, 4)],
            &[],
        );
        assert_eq!((n.rivers, n.cells), (1, 1));
        assert_eq!(n.max_m, 1.0);
        assert_eq!((dir[4], ground[8]), (2, 5.0));
        // Where no cell beside the entry may be carved down to its level, the route would
        // climb: no reroute, the river ends at its entry.
        ground[8] = 6.0;
        let floor: Vec<f32> = ground.iter().map(|&g| g - 0.5).collect();
        let mut dir = vec![0u8; 16];
        (dir[4], dir[5], dir[6], dir[8]) = (4, 4, 6, 2);
        let n = reroute_inflows(
            dims,
            1,
            &mut ground,
            &flags,
            &[0; 16],
            &back,
            &mut dir,
            &[(0, 4)],
            &floor,
        );
        assert_eq!((n.rivers, n.stuck.as_slice()), (0, &[4][..]));
    }

    #[test]
    fn the_first_free_row_starts_as_the_previous_bands_ground() {
        // 2 x 4, one taken row; the previous band's ground over the free rows is 0, this
        // band's 10: the first free row starts as that band's, blending to this band's own.
        let dims = Dims { w: 2, h: 4 };
        let mut ground = vec![10.0f32; 8];
        let halo = vec![0.0f32; 4];
        blend_seam(dims, 1, &mut ground, &halo, &[]);
        assert_eq!(ground[2], 0.0);
        let k = 1.0 - 1.0 / SEAM_BLEND_ROWS as f32;
        assert!((ground[4] - (1.0 - k) * 10.0).abs() < 1e-5);
        assert_eq!(ground[6], 10.0, "past the previous band's rows");
        // A channel that band cut in its halo comes in only down to the seam limit.
        let mut ground = vec![10.0f32; 8];
        blend_seam(dims, 1, &mut ground, &halo, &[9.0, 12.0]);
        assert_eq!((ground[2], ground[3]), (9.0, 10.0));
    }

    #[test]
    fn carving_stops_at_depth_and_at_the_seam_side_slope() {
        // 40 x 40, one taken row: dry at 100 m but for a river (bed 90 m) at column 0; this
        // band's ground is 100 m, the side slope 1 m a cell, cuts at most 30 m. Below taken
        // dry ground a cut goes one side slope down a row; beside the river down towards its
        // bed; far from the seam to the depth limit.
        let (w, h) = (40, 40);
        let dims = Dims { w, h };
        let mut ground = vec![100.0f32; w * h];
        ground[0] = 90.0;
        let (floor, seam) = ground_floor(dims, 1, &ground, |i| i == 0, |_| None, 30.0, 1.0, 1.0);
        assert!(
            floor[..w].iter().all(|f| f.is_infinite()),
            "taken cells are never carved"
        );
        let at = |x: usize, y: usize| floor[y * w + x];
        assert!(
            (at(30, 1) - 99.0).abs() < 1e-4,
            "below dry ground: one side slope"
        );
        assert!((at(30, 3) - 97.0).abs() < 1e-4);
        assert!(
            (at(0, 1) - 91.0).abs() < 1e-4,
            "below the river: its bed plus the slope"
        );
        assert_eq!(at(30, 39), 70.0, "far from the seam: the depth limit");
        assert_eq!(seam[30], at(30, 1));
    }
}
