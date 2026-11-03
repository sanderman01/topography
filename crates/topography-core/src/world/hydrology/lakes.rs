//! Depressions of the filled surface, and which of them stay as lakes or dry basins.

use super::flood::{Dims, FIXED, OUTLET, RIVER, SEA, receiver};
use super::water::Table;

/// Judgment constants: the smallest kept lake, and dry basins (compact and deep, as the
/// Python tarns and dry hollows).
const LAKE_MIN_DEPTH_M: f32 = 1.0;
const LAKE_MIN_AREA_M2: f64 = 4096.0;
const DRY_MIN_DEPTH_M: f32 = 2.0;
const DRY_MAX_ELONGATION: f64 = 4.0;

/// One closed depression: a connected region of the filled surface above the ground.
pub(super) struct Depression {
    /// Rim cell the depression spills over, at its level.
    pub outlet: u32,
    pub level: f32,
    pub cells: u32,
    /// Sum of depth over cells (m · cells).
    pub volume: f64,
    pub max_depth: f32,
    /// Most common class of its cells.
    pub class: u8,
    /// Longest side of its bounding box, in cells.
    pub span: u32,
    /// Drainage area into it, km².
    pub inflow_km2: f32,
    /// Kept as this kind whatever the budget (a coarse dry basin on a finer level).
    pub forced: Option<Kind>,
    /// Never kept, even when forced (on a finer level: it holds or touches a coarse lake or
    /// river path).
    pub excluded: bool,
}

/// Label each depression (8-connected cells with filled > ground) in `label` (id = index + 1).
pub(super) fn find(
    dims: Dims,
    ground: &[f32],
    filled: &[f32],
    dir: &[u8],
    classes: Option<&[u8]>,
    label: &mut [u32],
) -> Vec<Depression> {
    let mut found = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut histogram = [0u32; 256];
    for start in 0..ground.len() {
        if label[start] != 0 || filled[start] <= ground[start] {
            continue;
        }
        let id = found.len() as u32 + 1;
        histogram.fill(0);
        let (mut cells, mut volume, mut max_depth) = (0u32, 0f64, 0f32);
        let (mut pit, mut outlet) = (start, None);
        let (mut y0, mut y1, mut x0, mut x1) = (usize::MAX, 0, usize::MAX, 0);
        label[start] = id;
        stack.push(start as u32);
        while let Some(c) = stack.pop() {
            let c = c as usize;
            let depth = filled[c] - ground[c];
            cells += 1;
            volume += depth as f64;
            max_depth = max_depth.max(depth);
            if ground[c] < ground[pit] {
                pit = c;
            }
            let (y, x) = (c / dims.w, c % dims.w);
            (y0, y1, x0, x1) = (y0.min(y), y1.max(y), x0.min(x), x1.max(x));
            if let Some(cls) = classes {
                histogram[cls[c] as usize] += 1;
            }
            if let Some(r) = receiver(dims, dir, c) {
                if filled[r] <= ground[r] {
                    outlet = Some(r);
                }
            }
            for (_, n) in dims.neighbours(c) {
                if label[n] == 0 && filled[n] > ground[n] {
                    label[n] = id;
                    stack.push(n as u32);
                }
            }
        }
        let class = (0..256)
            .max_by_key(|&c| (histogram[c], std::cmp::Reverse(c)))
            .unwrap_or(0) as u8;
        found.push(Depression {
            // The flood always enters a depression from a rim cell (see `fill`).
            outlet: outlet.unwrap_or(pit) as u32,
            level: filled[start],
            cells,
            volume,
            max_depth,
            class,
            span: ((y1 - y0).max(x1 - x0) + 1) as u32,
            inflow_km2: 0.0,
            forced: None,
            excluded: false,
        });
    }
    found
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Lake,
    /// Closed and dry; flow passes through at its level.
    Dry,
    /// Closed and dry; flow ends in it (endorheic).
    Sink,
}

/// A depression kept as a lake or a dry basin.
pub(super) struct Basin {
    pub kind: Kind,
    pub level: f32,
    /// Cell where the routed flow leaves the basin (set after breaching).
    pub exit: u32,
    /// Became a lake because a river flows into it.
    pub filled_by_river: bool,
    /// Kept beyond the class budget because breaching it would cut too deep.
    pub over_budget: bool,
    /// A glacier basin: never a lake, never breached.
    pub ice: bool,
    /// Body id: a coarse lake's id on finer levels, else 0 until ids are assigned.
    pub id: u32,
}

/// No exit known (a coarse lake whose flow leaves outside the grid).
pub(super) const NO_EXIT: u32 = u32::MAX;

impl Basin {
    /// A basin kept beyond the budget because breaching it would cut too deep.
    pub fn kept(kind: Kind, level: f32, exit: u32) -> Self {
        Self {
            kind,
            level,
            exit,
            filled_by_river: false,
            over_budget: true,
            ice: false,
            id: 0,
        }
    }
}

/// Glacier class (legend v3): its basins hold ice, not lakes.
pub(super) const GLACIER: u8 = 10;
/// Classes with at most this lake share are arid: deep basins there stay dry.
pub(super) const ARID_LAKE_SHARE: f64 = 0.002;

/// Keep the largest depressions (by volume) as lakes within each class's `lake_share` of its
/// land, then the deepest compact ones of dry classes as dry basins within `dry_share`, if no
/// river flows in. Depressions belong to their most common class; glacier depressions all stay
/// as dry ice basins. Returns the basins and, per depression, its basin index or none.
pub(super) fn select(
    deps: &[Depression],
    land_by_class: &[u64; 256],
    table: &Table,
    cell_m2: f64,
    river_km2: f64,
) -> (Vec<Basin>, Vec<u32>) {
    let budget = |share: fn(&super::water::Water) -> f64| -> Vec<f64> {
        (0..256)
            .map(|c| share(table.get(c as u8)) * land_by_class[c] as f64)
            .collect()
    };
    let mut kind: Vec<Option<Kind>> = (0..deps.len())
        .map(|k| {
            let d = &deps[k];
            let glacier = (d.class == GLACIER).then_some(Kind::Dry);
            d.forced.or(glacier).filter(|_| !d.excluded)
        })
        .collect();

    let mut lake_budget = budget(|w| w.lake_share);
    let mut order: Vec<usize> = (0..deps.len())
        .filter(|&k| {
            kind[k].is_none()
                && !deps[k].excluded
                && deps[k].max_depth >= LAKE_MIN_DEPTH_M
                && deps[k].cells as f64 * cell_m2 >= LAKE_MIN_AREA_M2
        })
        .collect();
    order.sort_by(|&a, &b| deps[b].volume.total_cmp(&deps[a].volume));
    for k in order {
        let left = &mut lake_budget[deps[k].class as usize];
        if deps[k].cells as f64 <= *left {
            *left -= deps[k].cells as f64;
            kind[k] = Some(Kind::Lake);
        }
    }

    let mut dry_budget = budget(|w| w.dry_share);
    let mut order: Vec<usize> = (0..deps.len())
        .filter(|&k| {
            let d = &deps[k];
            kind[k].is_none()
                && !d.excluded
                && dry_budget[d.class as usize] > 0.0
                && d.max_depth >= DRY_MIN_DEPTH_M
                && (d.span as f64).powi(2) / d.cells as f64 <= DRY_MAX_ELONGATION
                && (d.inflow_km2 as f64) < river_km2
        })
        .collect();
    order.sort_by(|&a, &b| deps[b].max_depth.total_cmp(&deps[a].max_depth));
    for k in order {
        let left = &mut dry_budget[deps[k].class as usize];
        if deps[k].cells as f64 <= *left {
            *left -= deps[k].cells as f64;
            kind[k] = Some(Kind::Dry);
        }
    }

    // Lakes first, so body ids follow lake size.
    let mut basins: Vec<Basin> = Vec::new();
    let mut map = vec![0u32; deps.len() + 1];
    for want in [Kind::Lake, Kind::Dry, Kind::Sink] {
        let mut ks: Vec<usize> = (0..deps.len()).filter(|&k| kind[k] == Some(want)).collect();
        ks.sort_by(|&a, &b| deps[b].volume.total_cmp(&deps[a].volume));
        for dep in ks {
            map[dep + 1] = basins.len() as u32 + 1;
            basins.push(Basin {
                kind: want,
                level: deps[dep].level,
                exit: deps[dep].outlet,
                filled_by_river: false,
                over_budget: false,
                ice: deps[dep].class == GLACIER,
                id: 0,
            });
        }
    }
    (basins, map)
}

/// Grow each lake over ground below its level that drains into it (cut by breaching next
/// to the lake), but not over `keep_out` cells (coarse river paths keep their course).
/// Returns the cells added.
pub(super) fn extend(
    dims: Dims,
    ground: &[f32],
    dir: &[u8],
    flags: &[u8],
    basin: &mut [u32],
    basins: &[Basin],
    keep_out: impl Fn(usize) -> bool,
) -> u64 {
    let lake = |b: u32| b != 0 && basins[b as usize - 1].kind == Kind::Lake;
    let mut stack: Vec<u32> = (0..basin.len())
        .filter(|&i| lake(basin[i]))
        .map(|i| i as u32)
        .collect();
    let mut added = 0;
    while let Some(c) = stack.pop() {
        let c = c as usize;
        let id = basin[c];
        let level = basins[id as usize - 1].level;
        for (_, n) in dims.neighbours(c) {
            if basin[n] == 0
                && flags[n] & (SEA | FIXED) == 0
                && !keep_out(n)
                && ground[n] < level
                && receiver(dims, dir, n).is_some_and(|r| basin[r] == id)
            {
                basin[n] = id;
                added += 1;
                stack.push(n as u32);
            }
        }
    }
    added
}

/// Receivers draining sink basin `id` (of `cells` cells) breadth first to its `exit`, the cell
/// the flood reached it from, so that it fills to that spill. None where it cannot: no exit
/// (a sink a river ends in rather than climb), or cells not reached from the exit (a pocket
/// below 0 m left inside a breached depression, or a basin split by trench widening).
pub(super) fn drain_to_exit(
    dims: Dims,
    basin: &[u32],
    id: u32,
    exit: u32,
    cells: usize,
) -> Option<Vec<(usize, u8)>> {
    if exit == NO_EXIT {
        return None;
    }
    let exit = exit as usize;
    let mut moves = Vec::with_capacity(cells);
    let mut queue = std::collections::VecDeque::from([exit]);
    let mut seen = std::collections::HashSet::from([exit]);
    while let Some(c) = queue.pop_front() {
        for (_, j) in dims.neighbours(c) {
            if basin[j] == id && seen.insert(j) {
                moves.push((j, dims.towards(j, c)));
                queue.push_back(j);
            }
        }
    }
    (moves.len() == cells).then_some(moves)
}

/// A sink filled to its own lowest spill (`fill_to_spill`).
pub(super) struct Spill {
    pub level: f32,
    /// Where the water leaves: the first cell beyond the spill.
    pub outlet: usize,
    /// Cells below the level, in 8-connected pieces (one lake each).
    pub pieces: Vec<Vec<usize>>,
    /// New receivers: the flooded region towards the outlet, sink cells above the level onto it.
    pub moves: Vec<(usize, u8)>,
    /// Sink cells that are dry ground now (at or above the level).
    pub dry: Vec<usize>,
    /// Sink cells it took (in a piece or dry); others stay in the sink.
    pub taken: usize,
    /// Other hollows (basin ids) it took cells of.
    pub merged: Vec<u32>,
    /// The lake (body id) it joins: the region lies below its level. Per piece, whether it lies
    /// beside that lake and joins it (else it is a lake of its own at that level).
    pub join: Option<(u32, Vec<bool>)>,
}

/// Min-heap entry by ground, then index.
struct Low(f32, usize);
impl PartialEq for Low {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o).is_eq()
    }
}
impl Eq for Low {}
impl Ord for Low {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Low {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// The seam step rule (`report::carving`) for lakes made near the seam.
pub(super) struct SeamStep {
    /// Rows copied from the previous band; 0: no seam.
    pub fixed_rows: usize,
    /// Per column, what hydrology changed in the previous band's last row (shown less input).
    pub change: Vec<f32>,
    /// The input of the first `SEAM_CHECK_ROWS` rows after the seam.
    pub raw: Vec<f32>,
    /// The side slope's rise per row.
    pub rise: f32,
    /// The previous band's water surface in its last row (NaN where dry).
    pub surface: Vec<f32>,
}

impl SeamStep {
    pub fn none() -> Self {
        Self {
            fixed_rows: 0,
            change: Vec::new(),
            raw: Vec::new(),
            rise: 0.0,
            surface: Vec::new(),
        }
    }
    /// Water (or ground) at `level` over cell `i` would differ across the seam by more than
    /// the rule allows.
    pub fn steps(&self, dims: Dims, i: usize, level: f32) -> bool {
        self.limit(dims, i)
            .is_some_and(|(mid, by)| (level - mid).abs() > by)
    }
    /// The highest the shown height of cell `i` may stand under the rule (+inf outside the
    /// rows it checks), less the report's tolerance.
    pub fn top(&self, dims: Dims, i: usize) -> f32 {
        self.limit(dims, i).map_or(f32::INFINITY, |(mid, by)| {
            mid + by - super::report::BANK_TOLERANCE_M
        })
    }
    /// The shown height with no step at cell `i` and the step allowed around it.
    fn limit(&self, dims: Dims, i: usize) -> Option<(f32, f32)> {
        let (y, x) = (i / dims.w, i % dims.w);
        if self.change.len() != dims.w || y < self.fixed_rows {
            return None;
        }
        let k = y - self.fixed_rows;
        match self.raw.get(k * dims.w + x) {
            Some(&raw) if k < super::report::SEAM_CHECK_ROWS => Some((
                raw + self.change[x],
                super::report::SEAM_STEP_M + self.rise * k as f32,
            )),
            _ => None,
        }
    }
}

/// Sink basin `id`, which a river enters at `start` but which cannot fill to its recorded exit
/// (a pocket below 0 m inside a breached depression, or a basin trench widening split): the
/// lake its piece holding `start` makes when filled to its own lowest spill. A priority flood
/// from the piece's lowest cell takes ground in rising order, and lower ground that drains back
/// in, until it reaches lower ground draining away to water, a dry pass-through basin or an
/// edge. Other such hollows it reaches, and lakes breaching left that drain in, fill with it
/// (`merged`); a lake at or above the level beside it takes it in (`join`). An error (the
/// reason) where that is not clean: the flood meets a kept sink, a climb's sink, a basin above
/// its level or other water draining in, or the grid edge; ground draining away (the previous
/// band's and `keep` cells, coarse river paths, included: they are walls, though a path below
/// the level may take the spill) or water lies below the level beside the lake; the previous
/// band's water its outflow runs into lies above the level (`seam`); or a sink cell above the
/// level cannot drain onto it.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_to_spill(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    dir: &[u8],
    id: u32,
    start: usize,
    keep: &dyn Fn(usize) -> bool,
    seam: &SeamStep,
) -> Result<Spill, &'static str> {
    use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
    const MAX_CELLS: usize = 1 << 22;
    const MAX_WALK: usize = 1 << 20;
    // The sink's piece holding `start`, and its lowest cell.
    let mut piece = HashSet::from([start]);
    let mut stack = vec![start];
    let mut pit = start;
    while let Some(a) = stack.pop() {
        if ground[a] < ground[pit] {
            pit = a;
        }
        for (_, j) in dims.neighbours(a) {
            if body[j] == id && piece.insert(j) {
                stack.push(j);
            }
        }
    }
    let closed = |i: usize| dims.is_edge(i);
    // Another hollow breaching or the climb pass left (such as the next pocket below 0 m) is
    // ground to fill, like this sink's own cells.
    let absorbed = std::cell::RefCell::new(HashSet::<u32>::new());
    let mut absorbed_cells: Vec<usize> = Vec::new();
    let hollow = |i: usize| {
        let b = body[i];
        b != 0 && flags[i] & SEA == 0 && {
            let basin = &basins[b as usize - 1];
            b == id
                || (basin.kind == Kind::Sink && basin.over_budget && basin.exit != NO_EXIT)
                || absorbed.borrow().contains(&b)
        }
    };
    let foreign = |i: usize| flags[i] & SEA != 0 || (body[i] != 0 && !hollow(i));
    let water = |i: usize| {
        flags[i] & SEA != 0 || (foreign(i) && basins[body[i] as usize - 1].kind == Kind::Lake)
    };
    let key = |i: usize| {
        if flags[i] & SEA != 0 {
            0.0
        } else if foreign(i) {
            basins[body[i] as usize - 1].level
        } else {
            ground[i]
        }
    };
    // Where the routing from `x` goes, through lakes and pass-through basins: back in
    // (Some(true)), away to the sea, an edge or a lake without an exit here (Some(false)), or
    // into another sink or a dead end (None).
    let walk = |inside: &HashSet<usize>, x: usize| {
        let mut c = x;
        for _ in 0..MAX_WALK {
            if inside.contains(&c) || hollow(c) {
                return Some(true);
            }
            if flags[c] & (SEA | OUTLET) != 0 || (flags[c] & FIXED != 0 && flags[c] & RIVER != 0) {
                return Some(false);
            }
            let kind = (body[c] != 0).then(|| basins[body[c] as usize - 1].kind);
            if kind == Some(Kind::Sink) {
                return None;
            }
            match receiver(dims, dir, c) {
                Some(r) => c = r,
                None => return (kind == Some(Kind::Lake)).then_some(false),
            }
        }
        None
    };
    let mut inside: HashSet<usize> = HashSet::new();
    let mut seen = HashSet::from([pit]);
    let mut heap = BinaryHeap::from([Low(ground[pit], pit)]);
    let mut level = f32::NEG_INFINITY;
    let (mut outlet, mut join) = (None, None);
    while let Some(Low(z, x)) = heap.pop() {
        if outlet.is_some() && z >= level {
            break;
        }
        if foreign(x) {
            // Water or a pass-through basin below the level takes the spill; afterwards
            // water beside the region that drains away is left as it is. A lake at or above
            // the level (all the region is below it, the rest of the rim higher) takes the
            // region in.
            let kind = (flags[x] & SEA == 0).then(|| basins[body[x] as usize - 1].kind);
            match walk(&inside, x) {
                Some(false) => {}
                // A lake breaching left that drains in (a lagoon whose outflow ends here)
                // is part of the hollow: its cells are ground to fill.
                Some(true)
                    if kind == Some(Kind::Lake) && basins[body[x] as usize - 1].over_budget =>
                {
                    let lake = body[x];
                    let mut cells = vec![x];
                    let mut k = 0;
                    let mut found = HashSet::from([x]);
                    while k < cells.len() {
                        for (_, j) in dims.neighbours(cells[k]) {
                            if body[j] == lake && flags[j] & SEA == 0 && found.insert(j) {
                                cells.push(j);
                            }
                        }
                        k += 1;
                    }
                    absorbed.borrow_mut().insert(lake);
                    absorbed_cells.extend(cells);
                    heap.push(Low(ground[x], x));
                    continue;
                }
                Some(true) => return Err("water beside it drains into it"),
                None => return Err("meets a kept sink or a dead end"),
            }
            match outlet {
                None if kind != Some(Kind::Sink) && z < level => outlet = Some(x),
                None if kind == Some(Kind::Lake) => {
                    (outlet, join, level) = (Some(x), Some(body[x]), z);
                    break;
                }
                None => return Err("meets a basin above the level"),
                Some(_) => {}
            }
            continue;
        }
        if z.to_bits() != ground[x].to_bits() {
            // Pushed at its lake's level before that lake was taken in.
            heap.push(Low(ground[x], x));
            continue;
        }
        if flags[x] & FIXED != 0 || keep(x) {
            // The previous band's cells are a wall, checked against the level below (no flow
            // goes back across the seam); so are coarse river paths, which keep their course,
            // unless one below the level takes the spill.
            if keep(x) && outlet.is_none() && z < level && walk(&inside, x) == Some(false) {
                outlet = Some(x);
            }
            continue;
        }
        if closed(x) {
            return Err("meets the grid edge");
        }
        if z < level || outlet.is_some() {
            match walk(&inside, x) {
                Some(true) => {}
                Some(false) if outlet.is_none() => {
                    outlet = Some(x);
                    continue;
                }
                Some(false) => continue,
                None => return Err("ground below the level drains into a kept sink or a dead end"),
            }
        }
        inside.insert(x);
        level = level.max(z);
        for (_, j) in dims.neighbours(x) {
            if seen.insert(j) {
                heap.push(Low(key(j), j));
            }
        }
        if inside.len() > MAX_CELLS {
            return Err("too large");
        }
    }
    let Some(outlet) = outlet else {
        return Err("no spill");
    };
    if absorbed_cells.iter().any(|i| !inside.contains(i)) {
        return Err("a lake draining in is only partly below the spill");
    }
    // The lake: region cells below the level; no ground draining away below it beside them.
    let below = |i: usize| inside.contains(&i) && ground[i] < level;
    let tolerance = super::report::BANK_TOLERANCE_M;
    let mut pieces = Vec::new();
    let mut taken: HashSet<usize> = HashSet::new();
    for &s in &inside {
        if !below(s) || !taken.insert(s) {
            continue;
        }
        let mut cells = vec![s];
        let mut k = 0;
        while k < cells.len() {
            for (_, j) in dims.neighbours(cells[k]) {
                if below(j) && taken.insert(j) {
                    cells.push(j);
                } else if inside.contains(&j) {
                    continue;
                } else if !water(j) && ground[j] < level - tolerance {
                    return Err("ground draining away lies below the level beside the lake");
                } else if water(j) && key(j) < level - tolerance {
                    // No rim between it and lower water: a step in the water surface.
                    return Err("water beside the lake lies below its level");
                }
            }
            k += 1;
        }
        cells.sort_unstable();
        pieces.push(cells);
    }
    if pieces.is_empty() {
        return Err("no ground below the level");
    }
    // The outflow crosses the dry region at the level to the outlet, then runs on as a river
    // whose water stands at about the lowest ground so far: the previous band's dry ground
    // beside it, which cannot be raised as a bank, must not lie below that water. Every dry
    // region cell counts (strict: not only those the outflow crosses); beyond the outlet, the
    // river while it is in the rows the seam checks.
    let low_fixed = |i: usize, water: f32| {
        dims.neighbours(i).any(|(_, j)| {
            flags[j] & (FIXED | SEA | RIVER) == FIXED
                && body[j] == 0
                && ground[j] < water - tolerance
        })
    };
    if inside.iter().any(|&i| low_fixed(i, level)) {
        return Err("the previous band's ground beside the outflow lies below the level");
    }
    if seam.fixed_rows > 0 {
        let end = seam.fixed_rows + super::report::SEAM_CHECK_ROWS;
        let (mut c, mut water) = (outlet, ground[outlet]);
        for _ in 0..4 * end {
            // Nor may the previous band's water it runs into stand above the level.
            if c / dims.w + 1 == seam.fixed_rows
                && seam
                    .surface
                    .get(c % dims.w)
                    .is_some_and(|&s| s > level + tolerance)
            {
                return Err("the previous band's water the outflow runs into lies above the level");
            }
            if c / dims.w >= end || flags[c] & (FIXED | SEA) != 0 || body[c] != 0 {
                break;
            }
            if low_fixed(c, water) {
                return Err("the previous band's ground beside the outflow lies below its water");
            }
            match receiver(dims, dir, c) {
                Some(r) if !inside.contains(&r) => (c, water) = (r, water.min(ground[r])),
                _ => break,
            }
        }
    }
    pieces.sort_unstable_by_key(|p| p[0]);
    // Pieces beside the lake it joins become part of it; others lakes of their own at its level.
    let join = join.map(|lake| {
        let touches = |p: &Vec<usize>| {
            p.iter().any(|&i| {
                dims.neighbours(i)
                    .any(|(_, j)| body[j] == lake && flags[j] & SEA == 0)
            })
        };
        (lake, pieces.iter().map(touches).collect::<Vec<bool>>())
    });
    // Receivers: breadth first from the outlet over the region.
    let mut moves = Vec::new();
    let mut done = HashSet::from([outlet]);
    let mut queue = VecDeque::from([outlet]);
    while let Some(c) = queue.pop_front() {
        for (_, j) in dims.neighbours(c) {
            if inside.contains(&j) && done.insert(j) {
                moves.push((j, dims.towards(j, c)));
                queue.push_back(j);
            }
        }
    }
    // Sink cells above the level drain onto the region, never uphill.
    let mut dry: Vec<usize> = inside
        .iter()
        .copied()
        .filter(|&i| body[i] != 0 && !below(i))
        .collect();
    let mut taken = inside.iter().filter(|&&i| body[i] == id).count();
    let mut merged: Vec<u32> = inside
        .iter()
        .map(|&i| body[i])
        .filter(|&b| b != 0 && b != id)
        .collect();
    merged.sort_unstable();
    merged.dedup();
    let mut from: HashMap<usize, usize> = HashMap::new();
    let mut heap = BinaryHeap::new();
    for &i in &piece {
        if !inside.contains(&i) {
            if let Some((_, p)) = dims.neighbours(i).find(|(_, j)| inside.contains(j)) {
                from.insert(i, p);
                heap.push(Low(ground[i], i));
            }
        }
    }
    while let Some(Low(z, x)) = heap.pop() {
        if z < ground[from[&x]] {
            return Err("a sink cell above the level lies in a hollow of its own");
        }
        moves.push((x, dims.towards(x, from[&x])));
        dry.push(x);
        for (_, j) in dims.neighbours(x) {
            if piece.contains(&j) && !inside.contains(&j) && !from.contains_key(&j) {
                from.insert(j, x);
                heap.push(Low(ground[j], j));
            }
        }
    }
    if from.len() + inside.iter().filter(|i| piece.contains(i)).count() != piece.len() {
        return Err("a sink cell above the level cannot drain onto the lake");
    }
    taken += from.len();
    moves.sort_unstable();
    dry.sort_unstable();
    Ok(Spill {
        level,
        outlet,
        pieces,
        moves,
        dry,
        taken,
        merged,
        join,
    })
}

/// Why `fill_river_sinks` leaves a sink whose lake would stand above a coarse path beside it.
const PATH_CAPPED: &str = "a coarse river path beside it is held below the level";
/// Why it leaves one whose outflow the carve floor holds above the level.
const FLOOR_HELD: &str = "the carve floor holds its outflow above the level";
/// Cells of an outflow checked against the carve floor, and the water depth over it there.
const OUTFLOW_CELLS: usize = 64;
const OUTFLOW_DEPTH_M: f32 = 0.01;

/// Which basins `fill_river_sinks` fills.
pub(super) struct RiverSinkRule<'a> {
    /// A finer level: sinks fill (from basin index `first`, the ones breaching and the climb
    /// pass left); on the trunk level dry pass-through basins do.
    pub fine: bool,
    pub first: usize,
    /// Rows copied from the previous band. A sink reaching the rows just after them does not
    /// fill to its recorded spill (a lake there would meet that band's dry ground), only to its
    /// own spill, where `fill_to_spill` checks that ground.
    pub fixed_rows: usize,
    pub river_km2: f32,
    /// Coarse river paths, which keep their course.
    pub keep: &'a dyn Fn(usize) -> bool,
    /// The highest a coarse path cell's water may stand (+inf elsewhere).
    pub cap: &'a dyn Fn(usize) -> f32,
    /// Lowest ground carving may reach per cell (`band::ground_floor`); empty: none.
    pub floor: &'a [f32],
    pub seam: &'a SeamStep,
}

/// What `fill_river_sinks` did.
#[derive(Default)]
pub(super) struct RiverSinks {
    /// Sinks filled as lakes on a finer level (their receivers changed).
    pub filled: usize,
    /// Sinks filled to their own spill, and the lakes they became.
    pub spills: u64,
    pub spill_lakes: u64,
    /// Lake pieces that joined a lake beside them instead.
    pub joined: u64,
    /// Sinks left dry that `fill_to_spill` could not fill, by reason.
    pub left: std::collections::BTreeMap<&'static str, u64>,
}

impl RiverSinks {
    /// Sums with a later pass; the sinks left are those of the last pass.
    pub fn add(&mut self, pass: RiverSinks) {
        self.filled += pass.filled;
        self.spills += pass.spills;
        self.spill_lakes += pass.spill_lakes;
        self.joined += pass.joined;
        self.left = pass.left;
    }
}

/// Basins a river (discharge `acc` from `river_km2`) runs into fill as lakes and overflow. On
/// the trunk level a dry pass-through basin (not ice). On finer levels a sink breaching left:
/// it had no receivers, and its cells now drain breadth first to where the flood reached it
/// (`drain_to_exit`); one whose cells do not all drain there, such as a pocket below 0 m left
/// inside a breached depression or a basin trench widening split, fills to its own lowest
/// spill (`fill_to_spill`). A sink a climbing or stuck river ends in (no exit) stays one, as
/// does one where neither is clean.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_river_sinks(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &mut [u32],
    basins: &mut Vec<Basin>,
    kept_basin: &mut Vec<bool>,
    dir: &mut [u8],
    acc: &[f32],
    rule: RiverSinkRule<'_>,
) -> RiverSinks {
    let mut inflow = vec![(0f32, 0usize); basins.len() + 1];
    let mut at_seam = vec![false; basins.len() + 1];
    let mut cells = vec![0usize; basins.len() + 1];
    // Per basin, the lowest cap of a coarse path beside it: a lake above that would stand
    // above the river beside it, whose water the cap holds lower.
    let path_cap = |i: usize| {
        dims.neighbours(i)
            .filter(|&(_, j)| (rule.keep)(j))
            .map(|(_, j)| (rule.cap)(j))
            .fold(f32::INFINITY, f32::min)
    };
    let mut capped = vec![f32::INFINITY; basins.len() + 1];
    for i in 0..body.len() {
        let b = body[i] as usize;
        if b != 0 {
            capped[b] = capped[b].min(path_cap(i));
        }
        if acc[i] > inflow[b].0 {
            inflow[b] = (acc[i], i);
        }
        cells[b] += 1;
        at_seam[b] |=
            rule.fixed_rows > 0 && i / dims.w < rule.fixed_rows + super::SEAM_LAKE_GAP_ROWS;
    }
    let mut done = RiverSinks {
        filled: 0,
        spills: 0,
        spill_lakes: 0,
        joined: 0,
        left: std::collections::BTreeMap::new(),
    };
    let mut merged: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for b in 0..cells.len() - 1 {
        let basin = &basins[b];
        let fills = if rule.fine {
            basin.kind == Kind::Sink && b >= rule.first
        } else {
            basin.kind == Kind::Dry
        };
        let id = b as u32 + 1;
        if !fills || basin.ice || inflow[b + 1].0 < rule.river_km2 || merged.contains(&id) {
            continue;
        }
        let drained = (rule.fine && !at_seam[b + 1])
            .then(|| drain_to_exit(dims, body, id, basin.exit, cells[b + 1]))
            .flatten();
        let tolerance = super::report::BANK_TOLERANCE_M;
        // The outflow's water stands on its carve floor at least (`rivers::extract`): above the
        // level, the lake's exit would stand off it.
        let held_up = |from: usize, level: f32| {
            let mut c = Some(from);
            for _ in 0..OUTFLOW_CELLS {
                let Some(x) = c.filter(|&x| flags[x] & (SEA | FIXED) == 0 && body[x] == 0) else {
                    return false;
                };
                // (Half the tolerance: the exit check allows it, f32 rounding included.)
                if rule
                    .floor
                    .get(x)
                    .is_some_and(|&f| f + OUTFLOW_DEPTH_M > level + 0.5 * tolerance)
                {
                    return true;
                }
                c = receiver(dims, dir, x);
            }
            false
        };
        if !rule.fine {
        } else if let Some(moves) = drained {
            if capped[b + 1] < basin.level - tolerance {
                *done.left.entry(PATH_CAPPED).or_insert(0) += 1;
                continue;
            }
            if basin.exit != NO_EXIT && held_up(basin.exit as usize, basin.level) {
                *done.left.entry(FLOOR_HELD).or_insert(0) += 1;
                continue;
            }
            for (j, d) in moves {
                dir[j] = d;
            }
        } else if basin.exit == NO_EXIT {
            // A climbing or stuck river ends here: the water arriving, not a hollow's spill.
            continue;
        } else {
            let start = inflow[b + 1].1;
            let spill = fill_to_spill(
                dims, ground, flags, body, basins, dir, id, start, rule.keep, rule.seam,
            );
            let spill = match spill {
                Ok(spill) => spill,
                Err(why) => {
                    *done.left.entry(why).or_insert(0) += 1;
                    continue;
                }
            };
            let low_path = |i: &usize| path_cap(*i) < spill.level - tolerance;
            if spill.pieces.iter().flatten().any(low_path) {
                *done.left.entry(PATH_CAPPED).or_insert(0) += 1;
                continue;
            }
            if held_up(spill.outlet, spill.level) {
                *done.left.entry(FLOOR_HELD).or_insert(0) += 1;
                continue;
            }
            for &(j, d) in &spill.moves {
                dir[j] = d;
            }
            for &j in &spill.dry {
                body[j] = 0;
            }
            // The first lake keeps the sink's id unless some of its cells stay a sink.
            let mut keep_id = spill.taken == cells[b + 1];
            for (k, piece) in spill.pieces.iter().enumerate() {
                let lake = match &spill.join {
                    Some((lake, beside)) if beside[k] => {
                        done.joined += 1;
                        *lake
                    }
                    _ if keep_id => {
                        keep_id = false;
                        done.spill_lakes += 1;
                        id
                    }
                    _ => {
                        basins.push(Basin::kept(Kind::Lake, spill.level, NO_EXIT));
                        kept_basin.push(true);
                        done.spill_lakes += 1;
                        basins.len() as u32
                    }
                };
                for &j in piece {
                    body[j] = lake;
                }
                if spill.join.as_ref().is_some_and(|(l, _)| *l == lake) {
                    continue;
                }
                let basin = &mut basins[lake as usize - 1];
                (basin.kind, basin.level, basin.exit) =
                    (Kind::Lake, spill.level, spill.outlet as u32);
                // Made by a river, not kept beyond a budget (later floods do not take it in).
                (basin.filled_by_river, basin.over_budget) = (true, false);
            }
            // A hollow it took cells of is not filled again (its counts are stale).
            merged.extend(&spill.merged);
            done.spills += 1;
            done.filled += 1;
            continue;
        }
        let basin = &mut basins[b];
        basin.kind = Kind::Lake;
        basin.filled_by_river = true;
        done.filled += usize::from(rule.fine);
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lake_does_not_grow_over_a_coarse_river_path() {
        // Cells 1 and 2 lie below the lake's level (cell 0) and drain into it; cell 1 is on a
        // coarse river path and stays out, which also keeps cell 2 out.
        let dims = Dims { w: 3, h: 1 };
        let ground = [0.0f32, 0.5, 0.6];
        let dir = [0u8, 3, 3];
        let mut basin = [1u32, 0, 0];
        let lakes = [Basin::kept(Kind::Lake, 1.0, 0)];
        assert_eq!(
            extend(dims, &ground, &dir, &[0; 3], &mut basin, &lakes, |i| i == 1),
            0
        );
        let mut basin = [1u32, 0, 0];
        assert_eq!(
            extend(dims, &ground, &dir, &[0; 3], &mut basin, &lakes, |_| false),
            2
        );
    }

    #[test]
    fn a_sink_fills_only_when_all_its_cells_drain_to_its_exit() {
        // Exit cell 0; sink cells 1 and 2 drain west to it.
        let dims = Dims { w: 4, h: 1 };
        let moves = drain_to_exit(dims, &[0, 1, 1, 0], 1, 0, 2).unwrap();
        assert_eq!(moves, [(1, 3), (2, 3)]);
        // A pocket not beside its exit (dry cell 1 between), and a sink with no exit.
        assert!(drain_to_exit(dims, &[0, 0, 1, 1], 1, 0, 2).is_none());
        assert!(drain_to_exit(dims, &[0, 1, 0, 0], 1, NO_EXIT, 1).is_none());
        // Partly reached: cell 1 is beside the exit, cell 3 behind dry cell 2.
        assert!(drain_to_exit(dims, &[0, 1, 0, 1], 1, 0, 2).is_none());
        // 8-way: on a 3 x 3 grid, cells 4 and 8 reach exit 0 only along the diagonal.
        let dims = Dims { w: 3, h: 3 };
        let body = [0, 0, 0, 0, 1, 0, 0, 0, 1];
        let moves = drain_to_exit(dims, &body, 1, 0, 2).unwrap();
        assert_eq!(moves, [(4, dims.towards(4, 0)), (8, dims.towards(8, 4))]);
    }

    /// A 7 x 5 grid with walls at 5 m around row 2: dry cell 1 (0.5 m), sink cells 2 and 3
    /// (-1 and -0.5 m, id 1), rim cell 4 (0.3 m), cell 5 (0.1 m) draining east into cell 6.
    /// The sink's exit is cell 0 of row 1, which it does not touch.
    struct Pocket {
        dims: Dims,
        ground: Vec<f32>,
        flags: Vec<u8>,
        body: Vec<u32>,
        dir: Vec<u8>,
        acc: Vec<f32>,
        basins: Vec<Basin>,
    }

    fn pocket(sea: bool) -> Pocket {
        let dims = Dims { w: 7, h: 5 };
        let at = |x: usize| 2 * dims.w + x;
        let mut ground = vec![5.0f32; 35];
        for (x, g) in [
            (1, 0.5),
            (2, -1.0),
            (3, -0.5),
            (4, 0.3),
            (5, 0.1),
            (6, -2.0),
        ] {
            ground[at(x)] = g;
        }
        let mut flags = vec![0u8; 35];
        if sea {
            flags[at(6)] = SEA;
        }
        let mut body = vec![0u32; 35];
        body[at(2)] = 1;
        body[at(3)] = 1;
        let mut dir = vec![0u8; 35];
        dir[at(5)] = dims.towards(at(5), at(6));
        dir[at(4)] = dims.towards(at(4), at(3));
        let mut acc = vec![0f32; 35];
        acc[at(3)] = 1.0;
        let basins = vec![Basin::kept(Kind::Sink, 0.5, dims.w as u32)];
        Pocket {
            dims,
            ground,
            flags,
            body,
            dir,
            acc,
            basins,
        }
    }

    fn fill(p: &mut Pocket, first: usize) -> RiverSinks {
        let mut kept = vec![true; p.basins.len()];
        let rule = RiverSinkRule {
            fine: true,
            first,
            fixed_rows: 0,
            river_km2: 0.05,
            keep: &|_| false,
            cap: &|_| f32::INFINITY,
            floor: &[],
            seam: &SeamStep::none(),
        };
        fill_river_sinks(
            p.dims,
            &p.ground,
            &p.flags,
            &mut p.body,
            &mut p.basins,
            &mut kept,
            &mut p.dir,
            &p.acc,
            rule,
        )
    }

    #[test]
    fn a_pocket_not_beside_its_exit_fills_to_its_own_spill() {
        let mut p = pocket(true);
        let w = p.dims.w;
        let at = |x: usize| 2 * w + x;
        let spill = fill_to_spill(
            p.dims,
            &p.ground,
            &p.flags,
            &p.body,
            &p.basins,
            &p.dir,
            1,
            at(3),
            &|_| false,
            &SeamStep::none(),
        )
        .unwrap();
        assert_eq!((spill.level, spill.outlet), (0.3, at(5)));
        assert_eq!(spill.pieces, [vec![at(2), at(3)]]);
        assert_eq!(spill.taken, 2);
        let done = fill(&mut p, 0);
        assert_eq!((done.filled, done.spills, done.spill_lakes), (1, 1, 1));
        let lake = &p.basins[0];
        assert_eq!((lake.kind, lake.level), (Kind::Lake, 0.3));
        assert!(lake.filled_by_river);
        // The lake drains east over the rim to the outlet.
        for x in 2..5 {
            assert_eq!(receiver(p.dims, &p.dir, at(x)), Some(at(x + 1)));
        }
    }

    fn spill_of(p: &Pocket, seam: &SeamStep) -> Result<Spill, &'static str> {
        let start = 2 * p.dims.w + 3;
        fill_to_spill(
            p.dims,
            &p.ground,
            &p.flags,
            &p.body,
            &p.basins,
            &p.dir,
            1,
            start,
            &|_| false,
            seam,
        )
    }

    #[test]
    fn a_spill_takes_in_the_next_hollow_and_joins_a_lake_beside_it() {
        // Cell 1 is another breach hollow (id 2) at -0.2 m: it fills with the pocket.
        let mut p = pocket(true);
        let w = p.dims.w;
        let at = |x: usize| 2 * w + x;
        let (a1, a2, a3, a5) = (at(1), at(2), at(3), at(5));
        p.ground[a1] = -0.2;
        p.body[a1] = 2;
        p.basins.push(Basin::kept(Kind::Sink, 0.5, 0));
        let spill = spill_of(&p, &SeamStep::none()).unwrap();
        assert_eq!(spill.merged, [2]);
        assert_eq!(spill.pieces, [vec![a1, a2, a3]]);
        let done = fill(&mut p, 0);
        assert_eq!((done.spills, p.body[a1]), (1, 1));
        // Cell 5 is a lake at 0.4 m draining to the sea: above the 0.3 m rim, it takes the
        // pocket and the rim in.
        let mut p = pocket(true);
        p.body[a5] = 2;
        p.basins.push(Basin::kept(Kind::Lake, 0.4, at(6) as u32));
        let spill = spill_of(&p, &SeamStep::none()).unwrap();
        assert_eq!((spill.level, spill.join), (0.4, Some((2, vec![true]))));
        assert_eq!(spill.pieces, [vec![a2, a3, at(4)]]);
    }

    #[test]
    fn a_spill_splits_into_lakes_and_leaves_an_unreached_sink_piece() {
        // Cell 3 lies at the 0.3 m level between the pit (cell 2) and cell 4 (-0.2 m, draining
        // back in): two lakes. The sink's other piece, cell 5 of row 4 at 4 m, stays a sink, so
        // both lakes get new ids.
        let mut p = pocket(true);
        let w = p.dims.w;
        let at = |x: usize| 2 * w + x;
        let other = 4 * w + 5;
        (p.ground[at(3)], p.ground[at(4)], p.ground[at(5)]) = (0.3, -0.2, 0.3);
        (p.body[at(3)], p.body[other], p.ground[other]) = (0, 1, 4.0);
        (p.acc[at(3)], p.acc[at(2)]) = (0.0, 1.0);
        p.dir[at(5)] = p.dims.towards(at(5), at(6));
        let spill = spill_of(&p, &SeamStep::none());
        let spill = spill.unwrap();
        assert_eq!(spill.pieces, [vec![at(2)], vec![at(4)]]);
        assert_eq!(spill.taken, 1);
        let done = fill(&mut p, 0);
        assert_eq!((done.spills, done.spill_lakes), (1, 2));
        assert_eq!((p.body[at(2)], p.body[at(4)], p.body[other]), (2, 3, 1));
        assert_eq!(p.basins[0].kind, Kind::Sink);
        assert_eq!((p.basins[1].kind, p.basins[2].level), (Kind::Lake, 0.3));
    }

    #[test]
    fn a_sink_cell_above_the_level_in_a_hollow_of_its_own_rejects_the_spill() {
        // Sink cells below the pit: 0.4 m beside it, then 0.35 m behind that.
        let mut p = pocket(true);
        let w = p.dims.w;
        let (x, y) = (3 * w + 2, 4 * w + 2);
        (p.ground[x], p.ground[y], p.body[x], p.body[y]) = (0.4, 0.35, 1, 1);
        assert_eq!(
            spill_of(&p, &SeamStep::none()).err(),
            Some("a sink cell above the level lies in a hollow of its own")
        );
        p.ground[y] = 0.45;
        assert_eq!(spill_of(&p, &SeamStep::none()).unwrap().dry, [x, y]);
    }

    #[test]
    fn a_spill_lake_deep_at_the_seam_is_no_step() {
        // Rows 0 and 1 are the previous band's, unchanged there; the 0.3 m lake over cell 2,
        // whose input lay at -6 m, is 6.3 m deep: water depth is no change of the land.
        let p = pocket(true);
        let w = p.dims.w;
        let mut seam = SeamStep {
            fixed_rows: 2,
            change: vec![0.0; w],
            raw: vec![0.0; 4 * w],
            rise: 0.7,
            surface: vec![f32::NAN; w],
        };
        seam.raw[2] = -6.0;
        assert!(spill_of(&p, &seam).is_ok());
    }

    #[test]
    fn a_spill_lake_does_not_stand_above_the_previous_band() {
        // Rows 0 and 1 come from the previous band: a wall, but its cell beside the pocket lies
        // at 0 m, below the 0.3 m spill.
        let mut p = pocket(true);
        let w = p.dims.w;
        for f in &mut p.flags[..2 * w] {
            *f |= FIXED;
        }
        let spill = |p: &Pocket| {
            fill_to_spill(
                p.dims,
                &p.ground,
                &p.flags,
                &p.body,
                &p.basins,
                &p.dir,
                1,
                2 * w + 3,
                &|_| false,
                &SeamStep::none(),
            )
        };
        assert!(spill(&p).is_ok());
        p.ground[w + 2] = 0.0;
        assert_eq!(
            spill(&p).err(),
            Some("ground draining away lies below the level beside the lake")
        );
        // Its cell beside the rim and the outlet (not the lake) at 0 m: the outflow would
        // stand above it.
        p.ground[w + 2] = 5.0;
        p.ground[w + 5] = 0.0;
        assert_eq!(
            spill(&p).err(),
            Some("the previous band's ground beside the outflow lies below the level")
        );
        // Beside the outlet only (0.1 m, its water): 0.2 m lies above that water, 0.05 m below.
        p.ground[w + 5] = 5.0;
        let seam = SeamStep {
            fixed_rows: 2,
            change: vec![0.0; w],
            raw: vec![0.0; 4 * w],
            rise: 0.7,
            surface: vec![f32::NAN; w],
        };
        p.ground[w + 6] = 0.2;
        assert!(spill_of(&p, &seam).is_ok());
        p.ground[w + 6] = 0.05;
        assert_eq!(
            spill_of(&p, &seam).err(),
            Some("the previous band's ground beside the outflow lies below its water")
        );
        // The outflow runs into the previous band's river above it: its water may not stand
        // above the 0.3 m level.
        p.ground[w + 6] = 5.0;
        p.flags[w + 5] |= RIVER;
        p.dir[2 * w + 5] = p.dims.towards(2 * w + 5, w + 5);
        let mut seam = seam;
        seam.surface[5] = 0.2;
        assert!(spill_of(&p, &seam).is_ok());
        seam.surface[5] = 0.5;
        assert_eq!(
            spill_of(&p, &seam).err(),
            Some("the previous band's water the outflow runs into lies above the level")
        );
    }

    #[test]
    fn a_rejected_river_sink_stays_a_sink() {
        // The spill would drain into a dead end (cell 6 is dry land without a receiver).
        let mut p = pocket(false);
        let (body, dir) = (p.body.clone(), p.dir.clone());
        let done = fill(&mut p, 0);
        assert_eq!((done.filled, done.spills), (0, 0));
        assert_eq!(p.basins[0].kind, Kind::Sink);
        assert!(!p.basins[0].filled_by_river);
        assert_eq!((p.body, p.dir), (body, dir));
        // A sink a climbing river ends in (no exit) stays one too.
        let mut p = pocket(true);
        p.basins[0].exit = NO_EXIT;
        let done = fill(&mut p, 0);
        assert_eq!((done.filled, p.basins[0].kind), (0, Kind::Sink));
        // And so does one kept by the budget (index below `first`).
        let mut p = pocket(true);
        let done = fill(&mut p, 1);
        assert_eq!((done.filled, p.basins[0].kind), (0, Kind::Sink));
    }
}
