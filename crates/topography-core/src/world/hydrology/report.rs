//! Validation checks, figures and the body list.

use super::flood::{CENTRE, Dims, FIXED, OUTLET, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind, NO_EXIT, SeamStep};
use super::rivers::{Band, End, River, cascade_side, entered_basin, hole, is_bank, owner};
use serde_json::{Map, Value, json};

/// Dry ground may sit this far below adjacent water (f32 rounding of carved rims). The repairs
/// that keep banks above water use it too.
pub(super) const BANK_TOLERANCE_M: f32 = 0.01;
/// f32 rounding at mountain heights: an exit river's water stands its minimum depth (the
/// tolerance) over a carve floor at the lake's level, give or take this.
const ROUNDING_M: f32 = 0.001;
/// A centreline bed may rise this much into the next centreline cell (f32 rounding).
const BED_TOLERANCE_M: f32 = 0.01;

pub(super) struct Context<'a> {
    pub dims: Dims,
    pub ground: &'a [f32],
    pub dir: &'a [u8],
    pub acc: &'a [f32],
    pub flags: &'a [u8],
    /// Basin id, or river id where `RIVER` is set.
    pub body: &'a [u32],
    pub surface: &'a [f32],
    pub basins: &'a [Basin],
    pub rivers: &'a [River],
    /// Channel cells beside the centreline, by cell.
    pub band: &'a [Band],
    pub cell_km2: f64,
    pub spacing_m: f64,
    /// Top and bottom rows are band edges (not the world's), where a lake may continue.
    pub open_rows: [bool; 2],
    /// Rows copied from the previous band.
    pub fixed_rows: usize,
    /// Lowest ground carving may reach per cell (`band::ground_floor`); empty: none.
    pub carve_floor: &'a [f32],
    /// Rows whose banks hold even ahead along a river's flow (`rivers::is_bank`).
    pub held: [usize; 2],
    /// The previous band's ground over this band's first row (`rivers::cascade_side`).
    pub halo: &'a [f32],
    /// The seam step rule (`lakes::SeamStep::top` caps bank raises on a cascade side).
    pub seam: &'a SeamStep,
}

impl Context<'_> {
    fn fixed(&self, i: usize) -> bool {
        self.flags[i] & FIXED != 0
    }
    fn sea(&self, i: usize) -> bool {
        self.flags[i] & SEA != 0
    }
    fn river(&self, i: usize) -> bool {
        self.flags[i] & RIVER != 0
    }
    fn centre(&self, i: usize) -> bool {
        self.flags[i] & CENTRE != 0
    }
    fn basin(&self, i: usize) -> Option<&Basin> {
        (!self.sea(i) && !self.river(i) && self.body[i] != 0)
            .then(|| &self.basins[self.body[i] as usize - 1])
    }
    fn lake(&self, i: usize) -> Option<u32> {
        self.basin(i)
            .filter(|b| b.kind == Kind::Lake)
            .map(|_| self.body[i])
    }
    fn wet(&self, i: usize) -> bool {
        self.sea(i) || self.river(i) || self.lake(i).is_some()
    }
}

/// Ground lowered by every step together (breaching, trench widening, stream power and
/// channels), against the input. Per-step figures are in `trenches`, `stream_power` and
/// `channels`.
#[derive(Default)]
pub(super) struct Carve {
    cells: u64,
    /// Cut more than 10 m.
    deep: u64,
    volume_m3: f64,
    max_m: f32,
    max_at: [usize; 2],
    /// Raised more than `RAISE_MAX_M`, and the most raised.
    pub raised_over: u64,
    pub raise_max_m: f32,
    pub raise_max_at: [usize; 2],
}

impl Carve {
    pub fn add(&mut self, y: usize, input: &[f32], carved: &[f32], spacing_m: f64) {
        for (x, (a, b)) in input.iter().zip(carved).enumerate() {
            if b - a > self.raise_max_m {
                (self.raise_max_m, self.raise_max_at) = (b - a, [x, y]);
            }
            self.raised_over += u64::from(b - a > RAISE_MAX_M);
            if b < a {
                self.cells += 1;
                if a - b > 10.0 {
                    self.deep += 1;
                }
                self.volume_m3 += (a - b) as f64 * spacing_m * spacing_m;
                if a - b > self.max_m {
                    (self.max_m, self.max_at) = (a - b, [x, y]);
                }
            }
        }
    }
}

/// Hard checks; every value must be 0. `guard_` checks hold by construction and only catch
/// bugs; the others test the water contract on the outputs.
pub(super) fn checks(c: &Context, cycles: u64) -> (Map<String, Value>, Map<String, Value>) {
    let n = c.ground.len();
    let mut undrained = cycles;
    let (mut uphill, mut not_above, mut rises, mut not_below_level) = (0u64, 0u64, 0u64, 0u64);
    let (mut below_lake, mut below_sea, mut below_river) = (0u64, 0u64, 0u64);
    let (mut river_below_lake, mut bed_rises, mut holes) = (0u64, 0u64, 0u64);
    let (mut join_steps, mut join_step_max) = (0u64, 0f32);
    let mut lake_range = vec![(f32::INFINITY, f32::NEG_INFINITY); c.basins.len() + 1];
    // Up to three failing cells per check, [column, row].
    let mut examples = Map::new();
    let mut note = |name: &str, i: usize| {
        let v = examples.entry(name).or_insert_with(|| json!([]));
        if let Some(v) = v.as_array_mut().filter(|v| v.len() < 3) {
            v.push(json!([i % c.dims.w, i / c.dims.w]));
        }
    };
    for i in 0..n {
        // Cells copied from the previous band were checked there; here only against the
        // cells beside them that this run made.
        let fixed = c.fixed(i);
        let r = receiver(c.dims, c.dir, i);
        let sink = c.basin(i).is_some_and(|b| b.kind == Kind::Sink);
        if r.is_none() && !c.sea(i) && c.flags[i] & (OUTLET | FIXED) == 0 && !sink {
            undrained += 1;
            note("guard_land_cells_not_draining_to_an_outlet", i);
        }
        // Only dry land must descend. Channel cells are under water whose surface is flat
        // across the section and never rises along the centreline, so their beds may hold
        // pools; centreline beds have their own check.
        let land = !c.wet(i) && c.basin(i).is_none() && !fixed;
        if let Some(r) = r {
            if land && !c.sea(r) && c.basin(r).is_none() && c.ground[r] > c.ground[i] {
                uphill += 1;
                note("land_cells_draining_uphill", i);
            }
        }
        let s = c.surface[i];
        if !s.is_nan() && s <= c.ground[i] {
            not_above += 1;
            note("wet_cells_with_surface_not_above_ground", i);
        }
        if let Some(l) = c.lake(i) {
            let range = &mut lake_range[l as usize];
            *range = (range.0.min(s), range.1.max(s));
            if c.ground[i] >= c.basins[l as usize - 1].level {
                not_below_level += 1;
                note("lake_cells_not_below_level", i);
            }
        }
        if c.centre(i) && !fixed {
            let below = match r {
                Some(r) if c.wet(r) => c.surface[r],
                _ => f32::NEG_INFINITY,
            };
            // A join into the previous band's water near the seam may step up a little.
            // Only into the previous band's river, where a taken bank (not ahead along the
            // flow) holds the water below it, as `rivers::extract` makes it.
            let held_down = c.dims.neighbours(i).any(|(_, k)| {
                c.fixed(k)
                    && !c.wet(k)
                    && !c.dims.ahead(c.dir[i], i, k)
                    && c.ground[k] + BANK_TOLERANCE_M < below
            });
            let join = r.is_some_and(|r| {
                c.fixed(r)
                    && c.river(r)
                    && c.fixed_rows > 0
                    && i / c.dims.w < c.fixed_rows + SEAM_CHECK_ROWS
            }) && held_down;
            if join && below > s && below <= s + SEAM_JOIN_STEP_M + BANK_TOLERANCE_M {
                join_steps += 1;
                join_step_max = join_step_max.max(below - s);
            } else if below > s {
                rises += 1;
                note("river_cells_rising_downstream", i);
            }
            // A bed held at its carve floor may rise (a pool under the water).
            let held = |r: usize| {
                c.carve_floor
                    .get(r)
                    .is_some_and(|&f| c.ground[r] <= f + BED_TOLERANCE_M)
            };
            if r.is_some_and(|r| {
                c.centre(r) && c.ground[r] > c.ground[i] + BED_TOLERANCE_M && !held(r)
            }) {
                bed_rises += 1;
                note("guard_river_beds_rising_downstream", i);
            }
        }
        if !c.wet(i) {
            if !fixed
                && hole(
                    c.dims, c.dir, c.flags, c.body, c.basins, c.band, c.ground, c.surface, i,
                )
                .is_some()
            {
                holes += 1;
                note("dry_cells_in_holes", i);
            }
            let g = c.ground[i] + BANK_TOLERANCE_M;
            let near = |j: usize| !fixed || !c.fixed(j);
            let next = |kind: fn(&Context, usize) -> bool| {
                c.dims
                    .neighbours(i)
                    .any(|(_, j)| near(j) && kind(c, j) && c.surface[j] > g)
            };
            if next(|c, j| c.lake(j).is_some()) {
                below_lake += 1;
                note("dry_cells_below_adjacent_lake", i);
            }
            if next(|c, j| c.sea(j)) {
                below_sea += 1;
                note("dry_cells_below_adjacent_sea", i);
            }
            // Banks as in `rivers::is_bank`: not the basin the river enters (see
            // `mouth_max_drop_m`), not ground ahead along the flow; beside the previous band's
            // river on its cascade side (`rivers::cascade_side`) up to what the seam step rule
            // allows there (`SeamStep::top`), as the bank raise caps it.
            if c.dims.neighbours(i).any(|(_, j)| {
                near(j)
                    && c.surface[j] > g
                    && (!c.fixed(j)
                        || c.seam.top(c.dims, i) > g
                        || !cascade_side(
                            c.dims,
                            c.dir[j],
                            c.surface,
                            c.ground,
                            c.fixed_rows,
                            c.halo,
                            j,
                            i,
                        ))
                    && (c.fixed(j) && !fixed && c.river(j)
                        || owner(c.flags, c.band, j).is_some_and(|o| {
                            let entered = entered_basin(c.dims, c.dir, c.flags, c.body, o);
                            is_bank(
                                c.dims, c.dir, c.flags, c.body, c.basins, entered, o, j, i, c.held,
                            )
                        }))
            }) {
                below_river += 1;
                note("dry_cells_below_adjacent_river", i);
            }
        }
        if c.river(i) && !fixed {
            let low = s + BANK_TOLERANCE_M;
            if c.dims
                .neighbours(i)
                .any(|(_, j)| c.lake(j).is_some() && c.surface[j] > low)
            {
                river_below_lake += 1;
                note("river_cells_below_adjacent_lake", i);
            }
        }
    }
    let several = lake_range
        .iter()
        .filter(|(lo, hi)| lo.is_finite() && lo != hi)
        .count();
    // Lake exits: an outflow river at the level (a sill), dry ground not above it.
    let exits_off = c
        .basins
        .iter()
        .filter(|b| b.kind == Kind::Lake && b.exit != NO_EXIT)
        .filter(|b| {
            let e = b.exit as usize;
            // An exit on a band edge row: the lake may go on beyond it.
            let edge = (c.open_rows[0] && e < c.dims.w) || (c.open_rows[1] && e >= n - c.dims.w);
            let off = if c.sea(e) || c.lake(e).is_some() || edge {
                false
            } else if c.river(e) {
                (c.surface[e] - b.level).abs() > BANK_TOLERANCE_M + ROUNDING_M
            } else {
                c.ground[e] > b.level + BANK_TOLERANCE_M
            };
            if off {
                note("lake_exits_off_level", e);
            }
            off
        })
        .count();
    let (pieces, split, _) = lake_pieces(c);
    for i in split {
        note("lakes_in_several_pieces", i);
    }
    let mut on_land = 0;
    for r in c
        .rivers
        .iter()
        .filter(|r| r.cells > 0 && r.end == End::Land)
    {
        on_land += 1;
        note("rivers_ending_on_dry_land", r.mouth as usize);
    }
    let mut m = Map::new();
    m.insert(
        "guard_land_cells_not_draining_to_an_outlet".into(),
        json!(undrained),
    );
    m.insert("guard_lakes_with_several_levels".into(), json!(several));
    m.insert("land_cells_draining_uphill".into(), json!(uphill));
    m.insert("lakes_in_several_pieces".into(), json!(pieces));
    m.insert("lake_cells_not_below_level".into(), json!(not_below_level));
    m.insert("lake_exits_off_level".into(), json!(exits_off));
    m.insert(
        "river_cells_below_adjacent_lake".into(),
        json!(river_below_lake),
    );
    m.insert(
        "wet_cells_with_surface_not_above_ground".into(),
        json!(not_above),
    );
    m.insert("river_cells_rising_downstream".into(), json!(rises));
    // Taken out as a figure by the caller.
    examples.insert(
        "seam_join_steps".into(),
        json!({"count": join_steps, "max_m": join_step_max}),
    );
    m.insert(
        "guard_river_beds_rising_downstream".into(),
        json!(bed_rises),
    );
    m.insert("dry_cells_in_holes".into(), json!(holes));
    m.insert("rivers_ending_on_dry_land".into(), json!(on_land));
    m.insert("dry_cells_below_adjacent_lake".into(), json!(below_lake));
    m.insert("dry_cells_below_adjacent_sea".into(), json!(below_sea));
    m.insert("dry_cells_below_adjacent_river".into(), json!(below_river));
    (m, examples)
}

/// Judgment constant: what hydrology changed (shown height less the input) may differ across
/// the seam by this much, plus the side slope's rise per row further in (`SEAM_CHECK_ROWS`).
pub(super) const SEAM_STEP_M: f32 = 5.0;
/// Rows after the seam the step rule compares with the previous band's last row.
pub(super) const SEAM_CHECK_ROWS: usize = 4;
/// Judgment constant: in those rows a river joining the previous band's water may stand this
/// far below it where a taken bank holds it down (a step up into the join, reported as
/// `seam_join_steps`); more fails `river_cells_rising_downstream`.
pub(super) const SEAM_JOIN_STEP_M: f32 = 1.0;
/// Judgment constant: a cell whose hydrology change lies this far below that of both cells
/// beside it (north and south, or west and east) is a slot.
pub(super) const SLOT_M: f32 = 5.0;
/// Judgment constant: a river bed may lie this far below the carve floor (about the deepest
/// channel); coarse river paths burned deeper fail here too.
pub(super) const RIVER_FLOOR_SLACK_M: f32 = 5.0;
/// Judgment constant: the most any step may raise ground above its input (fills, banks, sills;
/// a backwater over a rise the carve floor keeps fills the valley behind it up to the rise).
pub(super) const RAISE_MAX_M: f32 = 60.0;

/// What shows at a cell: the water surface where wet, else the ground.
fn shown(ground: &[f32], surface: &[f32], i: usize) -> f32 {
    if surface[i].is_finite() {
        surface[i]
    } else {
        ground[i]
    }
}

/// Slots found while streaming the input rows (`add`): cells whose change (shown less input)
/// lies `SLOT_M` below both neighbours north and south or west and east, over the rows after
/// `fixed_rows`; those in the first `SEAM_CHECK_ROWS` rows are seam slots. Also the input of
/// the rows the seam rule needs (the previous band's last row, then `SEAM_CHECK_ROWS`).
pub(super) struct Slots {
    w: usize,
    fixed_rows: usize,
    rows: [(usize, Vec<f32>); 2],
    pub raw_seam: Vec<f32>,
    pub cells: u64,
    pub seam_cells: u64,
    pub max_m: f32,
    pub seam_max_m: f32,
    pub at: Vec<[usize; 2]>,
}

impl Slots {
    pub fn new(w: usize, fixed_rows: usize) -> Self {
        Slots {
            w,
            fixed_rows,
            rows: [(usize::MAX, Vec::new()), (usize::MAX, Vec::new())],
            raw_seam: Vec::new(),
            cells: 0,
            seam_cells: 0,
            max_m: 0.0,
            seam_max_m: 0.0,
            at: Vec::new(),
        }
    }

    /// Input row `y` (rows in order) against the final `ground` and `surface`.
    pub fn add(&mut self, y: usize, input: &[f32], ground: &[f32], surface: &[f32]) {
        let w = self.w;
        if self.fixed_rows > 0 && y + 1 >= self.fixed_rows && y < self.fixed_rows + SEAM_CHECK_ROWS
        {
            self.raw_seam.extend_from_slice(input);
        }
        let d: Vec<f32> = (0..w)
            .map(|x| shown(ground, surface, y * w + x) - input[x])
            .collect();
        let [(ya, a), (yb, b)] = std::mem::replace(
            &mut self.rows,
            [(usize::MAX, Vec::new()), (usize::MAX, Vec::new())],
        );
        // Row yb is the middle one when rows ya, yb, y are consecutive.
        if ya != usize::MAX && yb == ya + 1 && y == yb + 1 && yb >= self.fixed_rows {
            let seam = yb < self.fixed_rows + SEAM_CHECK_ROWS;
            for x in 0..w {
                let ns = a[x].min(d[x]) - b[x];
                let we = if x > 0 && x + 1 < w {
                    b[x - 1].min(b[x + 1]) - b[x]
                } else {
                    0.0
                };
                let by = ns.max(we);
                if by > SLOT_M {
                    self.cells += 1;
                    self.max_m = self.max_m.max(by);
                    if seam {
                        self.seam_cells += 1;
                        self.seam_max_m = self.seam_max_m.max(by);
                    }
                    if self.at.len() < 3 {
                        self.at.push([x, yb]);
                    }
                }
            }
        }
        self.rows = [(yb, b), (y, d)];
    }
}

/// Carving checks (each must be 0): free dry ground below its carve `floor` (cut deeper than
/// the limit, or leaving a step at the seam); river beds more than `RIVER_FLOOR_SLACK_M` below
/// it; ground raised more than `RAISE_MAX_M` (`carve`); seam steps: across the seam
/// (`fixed_rows`) what hydrology changed (shown less input, `slots.raw_seam`) differs by more
/// than `SEAM_STEP_M` plus the side slope per row, over the first `SEAM_CHECK_ROWS` rows, except
/// where both cells are water of one body (`body_id`); and slots in those rows. With a cell
/// of each and the extremes.
pub(super) fn carving(
    c: &Context,
    floor: &[f32],
    held_rivers: &[f32],
    fixed_rows: usize,
    body_id: &dyn Fn(usize) -> u32,
    carve: &Carve,
    slots: &Slots,
) -> (Map<String, Value>, Value) {
    let w = c.dims.w;
    let cell = |i: usize| json!([i % w, i / w]);
    let (mut below, mut deepest, mut below_at) = (0u64, 0f32, Vec::new());
    let (mut beds, mut beds_deepest, mut beds_at) = (0u64, 0f32, Vec::new());
    for (i, &f) in floor.iter().enumerate() {
        if c.fixed(i) {
            continue;
        }
        let by = f - c.ground[i];
        // Channel beds may lie below it by about their depth; lakes and the sea are not carved.
        // In the core's last rows their floor there (`held_rivers`, the same since rivers keep
        // the side slope too).
        if c.river(i) {
            let f = (c.held[0] * w..c.held[1] * w)
                .contains(&i)
                .then(|| held_rivers.get(i - c.held[0] * w).copied())
                .flatten()
                .unwrap_or(f);
            let by = f - c.ground[i];
            if by > RIVER_FLOOR_SLACK_M {
                beds += 1;
                beds_deepest = beds_deepest.max(by);
                if beds_at.len() < 3 {
                    beds_at.push(cell(i));
                }
            }
            continue;
        }
        if c.wet(i) {
            continue;
        }
        if by > BANK_TOLERANCE_M {
            below += 1;
            deepest = deepest.max(by);
            if below_at.len() < 3 {
                below_at.push(cell(i));
            }
        }
    }
    let (mut steps, mut step_at, mut step_max) = (0u64, Vec::new(), 0f32);
    let rise = super::erosion::SIDE_SLOPE_DEG.to_radians().tan() * c.spacing_m as f32;
    let raw = &slots.raw_seam;
    if fixed_rows >= 1 && raw.len() >= 2 * w {
        // A lake's or the sea's depth is no change of the land: their ground counts there.
        let d = |k: usize, x: usize| {
            let i = (fixed_rows - 1 + k) * w + x;
            let still = c.sea(i) || c.lake(i).is_some();
            let h = if still {
                c.ground[i]
            } else {
                shown(c.ground, c.surface, i)
            };
            h - raw[k * w + x]
        };
        let rows = (raw.len() / w).min(c.dims.h + 1 - fixed_rows);
        for x in 0..w {
            let a = (fixed_rows - 1) * w + x;
            let mut worst = 0f32;
            for k in 1..rows {
                let b = (fixed_rows - 1 + k) * w + x;
                if c.wet(a) && c.wet(b) && body_id(a) == body_id(b) {
                    continue;
                }
                let step = (d(k, x) - d(0, x)).abs();
                if step > SEAM_STEP_M + rise * (k - 1) as f32 {
                    worst = worst.max(step);
                }
            }
            if worst > 0.0 {
                steps += 1;
                step_max = step_max.max(worst);
                if step_at.len() < 3 {
                    step_at.push(cell(fixed_rows * w + x));
                }
            }
        }
    }
    let mut m = Map::new();
    m.insert("ground_below_carve_floor".into(), json!(below));
    m.insert("river_beds_below_carve_floor".into(), json!(beds));
    m.insert("ground_raised_over_limit".into(), json!(carve.raised_over));
    m.insert("seam_steps".into(), json!(steps));
    m.insert("seam_slots".into(), json!(slots.seam_cells));
    let figures = json!({
        "ground_below_carve_floor_max_m": deepest,
        "ground_below_carve_floor_at_cells": below_at,
        "river_beds_below_carve_floor_max_m": beds_deepest,
        "river_beds_below_carve_floor_at_cells": beds_at,
        "ground_raised_max_m": carve.raise_max_m,
        "ground_raised_max_at_cell": carve.raise_max_at,
        "seam_step_max_m": step_max,
        "seam_steps_at_columns": step_at,
        "seam_slot_max_m": slots.seam_max_m,
        "slots": slots.cells,
        "slot_max_m": slots.max_m,
        "slots_at_cells": slots.at,
    });
    (m, figures)
}

/// Lakes whose cells form more than one 8-connected piece. A piece touching a band edge row
/// may join the rest beyond it and does not count, nor does a piece of the previous band's
/// cells reaching no further than the seam blend. Also a cell of each extra piece, and of each
/// such seam piece of a lake that has other pieces (split at the seam, shown as a figure).
fn lake_pieces(c: &Context) -> (usize, Vec<usize>, Vec<usize>) {
    let n = c.ground.len();
    let mut seen = vec![false; n];
    let mut pieces = vec![0u32; c.basins.len() + 1];
    let mut all = vec![0u32; c.basins.len() + 1];
    let mut taken_at: Vec<(u32, usize)> = Vec::new();
    let mut extra = Vec::new();
    let mut stack = Vec::new();
    let open = |i: usize| (c.open_rows[0] && i < c.dims.w) || (c.open_rows[1] && i >= n - c.dims.w);
    for i in 0..n {
        let Some(id) = c.lake(i).filter(|_| !seen[i]) else {
            continue;
        };
        seen[i] = true;
        stack.push(i);
        // A piece of the previous band's cells, and of this band's only within the seam blend
        // (where the ground is that band's), was joined through that band's halo, which this
        // band made again; it does not count.
        let blend_end = (c.fixed_rows + super::band::SEAM_BLEND_ROWS) * c.dims.w;
        let (mut edge, mut fixed, mut beyond) = (false, false, false);
        while let Some(a) = stack.pop() {
            edge |= open(a);
            fixed |= c.fixed(a);
            beyond |= a >= blend_end;
            for (_, j) in c.dims.neighbours(a) {
                if !seen[j] && c.lake(j) == Some(id) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        all[id as usize] += 1;
        let taken = fixed && !beyond;
        if taken && !edge {
            taken_at.push((id, i));
        }
        if !edge && !taken {
            pieces[id as usize] += 1;
            if pieces[id as usize] > 1 {
                extra.push(i);
            }
        }
    }
    let at_seam = taken_at
        .into_iter()
        .filter(|&(id, _)| all[id as usize] > 1)
        .map(|(_, i)| i)
        .collect();
    (pieces.iter().filter(|&&p| p > 1).count(), extra, at_seam)
}

/// Dry cells below an adjacent river surface that the bank check lets pass only because
/// they lie ahead along the flow (cascades): count, more than 3 m, deepest and where.
fn ahead_exposure(c: &Context) -> Value {
    let (mut cells, mut over3, mut max, mut at) = (0u64, 0u64, 0f32, 0usize);
    for i in (0..c.ground.len()).filter(|&i| !c.wet(i)) {
        let g = c.ground[i] + BANK_TOLERANCE_M;
        let mut exposed = 0f32;
        for (_, j) in c.dims.neighbours(i) {
            let Some(o) = owner(c.flags, c.band, j).filter(|_| c.surface[j] > g) else {
                continue;
            };
            let entered = entered_basin(c.dims, c.dir, c.flags, c.body, o);
            let in_entered = c.body[i] != 0 && c.body[i] == entered;
            if !in_entered && c.dims.ahead(c.dir[o], j, i) {
                exposed = exposed.max(c.surface[j] - c.ground[i]);
            }
        }
        if exposed > 0.0 {
            cells += 1;
            over3 += u64::from(exposed > 3.0);
            if exposed > max {
                (max, at) = (exposed, i);
            }
        }
    }
    json!({
        "cells": cells,
        "over_3m": over3,
        "max_m": max,
        "max_at_cell": [at % c.dims.w, at / c.dims.w],
    })
}

/// River ends (centreline cells of this run whose receiver is a dry sink cell): into a hollow
/// this level left (`over_budget`: breaching, a climb or a stuck inflow), with examples, and
/// into a kept dry basin (endorheic by design).
fn dry_sink_ends(c: &Context) -> Value {
    let (mut hollow, mut kept) = (Vec::new(), 0u64);
    let mut sinks = std::collections::HashSet::new();
    for i in 0..c.ground.len() {
        if !c.centre(i) || c.fixed(i) {
            continue;
        }
        let Some(r) = receiver(c.dims, c.dir, i) else {
            continue;
        };
        match c.basin(r).filter(|b| b.kind == Kind::Sink) {
            Some(b) if b.over_budget => {
                hollow.push(i);
                sinks.insert(c.body[r]);
            }
            Some(_) => kept += 1,
            None => {}
        }
    }
    let w = c.dims.w;
    json!({
        "count": hollow.len(),
        "sinks": sinks.len(),
        "at_cells": hollow.iter().take(8).map(|&i| [i % w, i / w]).collect::<Vec<_>>(),
        "into_kept_dry_basins": kept,
    })
}

pub(super) fn figures(c: &Context, carve: &Carve, extended: u64) -> Map<String, Value> {
    let n = c.ground.len();
    let sea = (0..n).filter(|&i| c.sea(i)).count();
    let lake_cells = (0..n).filter(|&i| c.lake(i).is_some()).count();
    let dry_cells = (0..n)
        .filter(|&i| c.basin(i).is_some_and(|b| b.kind != Kind::Lake))
        .count();
    let river_cells = (0..n).filter(|&i| c.river(i)).count();
    // River surface above the floor of the dry basin it enters, beside the mouth.
    let mouth_drop = (0..n)
        .filter(|&i| c.river(i))
        .filter_map(|i| {
            let b = entered_basin(c.dims, c.dir, c.flags, c.body, i);
            (b != 0 && c.basins[b as usize - 1].kind != Kind::Lake).then(|| {
                c.dims
                    .neighbours(i)
                    .filter(|&(_, j)| c.basin(j).is_some() && c.body[j] == b)
                    .map(|(_, j)| c.surface[i] - c.ground[j])
                    .fold(0f32, f32::max)
            })
        })
        .fold(0f32, f32::max);
    let count = |f: fn(&Basin) -> bool| c.basins.iter().filter(|b| f(b)).count();
    let land = (n - sea) as f64;
    let here = || c.rivers.iter().filter(|r| r.cells > 0);
    let ends = |e: fn(&End) -> bool| here().filter(|r| e(&r.end)).count();
    let km2 = |cells: usize| cells as f64 * c.cell_km2;
    let (_, _, at_seam) = lake_pieces(c);
    let w = c.dims.w;
    let value = json!({
        // Not a check: this band cannot change the previous band's cells.
        "lake_pieces_split_at_seam": {
            "count": at_seam.len(),
            "at_cells": at_seam.iter().take(3).map(|&i| [i % w, i / w]).collect::<Vec<_>>(),
        },
        "cells": n,
        "spacing_m": c.spacing_m,
        "sea_km2": km2(sea),
        "sea_share": sea as f64 / n as f64,
        "land_km2": km2(n - sea),
        "lakes": count(|b| b.kind == Kind::Lake),
        "lake_km2": km2(lake_cells),
        "lake_share_of_land": lake_cells as f64 / land.max(1.0),
        "over_budget_lakes": count(|b| b.kind == Kind::Lake && b.over_budget),
        "lakes_filled_by_river": count(|b| b.filled_by_river),
        "lake_cells_added_below_level": extended,
        "dry_basins": count(|b| b.kind != Kind::Lake),
        "over_budget_sinks": count(|b| b.kind == Kind::Sink),
        "ice_basins": count(|b| b.ice),
        "dry_basin_km2": km2(dry_cells),
        "total_lowering": {
            "cells": carve.cells,
            "cells_over_10m": carve.deep,
            "volume_m3": carve.volume_m3.round(),
            "max_m": carve.max_m,
            "max_at_cell": carve.max_at,
        },
        "river_ahead_exemptions": ahead_exposure(c),
        "river_systems": here().count(),
        "coarse_river_systems": here().filter(|r| r.coarse).count(),
        "river_cells": river_cells,
        "river_centreline_cells": (0..n).filter(|&i| c.centre(i)).count(),
        "river_length_km": c.rivers.iter().map(|r| r.length_m).sum::<f64>() / 1000.0,
        "rivers_to_rivers": ends(|e| matches!(e, End::Joins(_))),
        "rivers_to_sea": ends(|e| *e == End::Sea),
        "rivers_to_lakes": ends(|e| matches!(e, End::Lake(_))),
        "rivers_to_sinks": ends(|e| matches!(e, End::Sink(_))),
        "rivers_ending_in_a_dry_sink": dry_sink_ends(c),
        "rivers_into_ice_basins": ends(|e| matches!(e, End::Dry(_))),
        "mouth_max_drop_m": mouth_drop,
        "rivers_to_grid_edge": ends(|e| *e == End::Edge),
        "max_drainage_km2": c.acc.iter().zip(c.flags).filter(|(_, f)| **f & SEA == 0)
            .map(|(a, _)| *a).fold(0f32, f32::max),
    });
    match value {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub(super) fn constants(river_km2: f64, breach_max_m: f32) -> Value {
    json!({
        "sea": "input ground below 0 m connected 8-way to the grid edge; surface 0",
        "river_start_km2": river_km2,
        "discharge": "drainage area weighted by precipitation / its land mean (km² at mean rain)",
        "channel": {
            "width_m": format!("{} * Q^0.5", super::rivers::WIDTH_A),
            "depth_m": format!("{} * Q^0.4", super::rivers::DEPTH_B),
            "q": "discharge in km² at mean rain",
            "section": "parabolic; each cell carved by its mean depth over the section, so a \
                channel narrower than a cell is one shallower cell",
            "surface": "the centreline ground, at most every dry bank beside or behind its \
                channel cells (not ground ahead along the flow) and every surface upstream",
            "mouth_cells": super::rivers::MOUTH_CELLS,
            "mouth": "the bed reaches below 0 m over the last mouth_cells cells and continues \
                into the seabed as far",
        },
        "stream_power": super::erosion::json(),
        "trench_widening": super::erosion::trench_json(),
        "hole_m": super::rivers::HOLE_M,
        "holes": "a dry cell (not in the basin a river enters) more than hole_m below every \
            adjacent water surface is raised to the lowest of them, with the dry cells \
            draining into it",
        "ahead_rule": "ground ahead of a river cell along the flow is the next \
            cross-section's bank, not this one's (a cascade); reported as \
            river_ahead_exemptions",
        "breach_max_m": breach_max_m,
        "deep_depressions": "kept beyond the budget: dry sinks in glacier and arid classes \
            (lake_share <= 0.002), else lakes at the spill lowered by breach_max_m, not below 0 m",
        "sub_zero_depressions": "at most breach_max_m deep with ground below 0 m: lakes at 0 m \
            (lagoons) in any class",
        "lake_exit": "an outflow river starts at the lake level; dry ground beside it below the \
            level is raised to it (a rock sill), with the dry cells draining into those",
        "lake_min_depth_m": 1.0,
        "lake_min_area_m2": 4096.0,
        "dry_basin_min_depth_m": 2.0,
        "dry_basin_max_elongation": 4.0,
        "lake_order": "largest volume first within each class's lake_share of its land",
        "dry_order": "deepest first within each dry class's dry_share, only without river inflow",
        "glacier_basins": "class 10 depressions stay dry ice basins, never lakes or breached",
        "bank_tolerance_m": BANK_TOLERANCE_M,
    })
}

/// `bodies.json`: sea, lakes and rivers (ids as in `water-body.u32`), then dry basins (ids
/// not in the raster) with their parent body (where their water goes; 0 for the grid edge).
/// Rivers without centreline cells here (dropped, or only in the previous band) and basins
/// without cells are left out.
pub(super) fn bodies(c: &Context, body_id: &dyn Fn(usize) -> u32) -> Value {
    let cell = |i: u32| json!([i as usize % c.dims.w, i as usize / c.dims.w]);
    let mut cells = vec![0u64; c.basins.len() + 1];
    let mut depth = vec![0f32; c.basins.len() + 1];
    let mut sea = 0u64;
    for i in 0..c.ground.len() {
        if c.sea(i) {
            sea += 1;
        } else if let Some(b) = c.basin(i) {
            let k = c.body[i] as usize;
            cells[k] += 1;
            depth[k] = depth[k].max(b.level - c.ground[i]);
        }
    }
    // Downstream from a lake exit to the first water body.
    let parent = |mut i: usize| loop {
        let id = body_id(i);
        if id != 0 {
            return id;
        }
        match receiver(c.dims, c.dir, i) {
            Some(r) => i = r,
            None => return 0,
        }
    };
    let mut list = vec![json!({
        "id": 1, "kind": "sea", "level_m": 0.0, "area_km2": sea as f64 * c.cell_km2,
    })];
    let mut dry = Vec::new();
    for (b, basin) in c.basins.iter().enumerate() {
        // Basins emptied here (fine basins that trench widening lowered) are left out.
        if cells[b + 1] == 0 {
            continue;
        }
        let common = json!({
            "id": basin.id,
            "level_m": basin.level,
            "area_km2": cells[b + 1] as f64 * c.cell_km2,
            "max_depth_m": depth[b + 1],
            "over_budget": basin.over_budget,
        });
        let mut entry = match common {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if basin.kind == Kind::Lake {
            let exit = (basin.exit != NO_EXIT).then_some(basin.exit);
            entry.insert("kind".into(), json!("lake"));
            entry.insert("exit_cell".into(), exit.map_or(Value::Null, cell));
            entry.insert("filled_by_river".into(), json!(basin.filled_by_river));
            entry.insert(
                "parent".into(),
                json!(exit.map_or(0, |e| parent(e as usize))),
            );
            list.push(Value::Object(entry));
        } else {
            entry.insert("kind".into(), json!("dry"));
            entry.insert("endorheic".into(), json!(basin.kind == Kind::Sink));
            entry.insert("ice".into(), json!(basin.ice));
            dry.push(Value::Object(entry));
        }
    }
    for river in c.rivers.iter().filter(|r| r.cells > 0) {
        let parent = match river.end {
            End::Sea => 1,
            End::Lake(b) | End::Sink(b) | End::Dry(b) if cells[b as usize] == 0 => 0,
            End::Lake(b) | End::Sink(b) | End::Dry(b) => c.basins[b as usize - 1].id,
            End::Joins(r) => c.rivers[r as usize - 1].id,
            End::Edge | End::Land => 0,
        };
        list.push(json!({
            "id": river.id,
            "kind": "river",
            "coarse": river.coarse,
            "mouth_cell": cell(river.mouth),
            "area_km2": river.wet_cells as f64 * c.cell_km2,
            "drainage_km2": river.drainage_km2,
            "length_km": river.length_m / 1000.0,
            "cells": river.cells,
            "parent": parent,
        }));
    }
    list.extend(dry);
    json!({
        "schema_version": 1,
        "kind": "water_bodies",
        "cells": "[column, row] from the north-west corner; a lake's exit_cell is the first cell \
            its routed flow enters outside it",
        "bodies": list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 x 5 world: a lake cell at (1, 1) drains into a river down column 2 to the sea row;
    /// (2, 2) is its exit, at the lake level.
    struct Scene {
        ground: Vec<f32>,
        dir: Vec<u8>,
        flags: Vec<u8>,
        body: Vec<u32>,
        surface: Vec<f32>,
        basins: Vec<Basin>,
        rivers: Vec<River>,
        band: Vec<Band>,
    }

    const W: usize = 5;
    type Mutation = fn(&mut Scene);
    fn at(y: usize, x: usize) -> usize {
        y * W + x
    }

    fn scene() -> Scene {
        let n = W * W;
        let mut s = Scene {
            ground: vec![10.0; n],
            dir: vec![2; n], // all drain south
            flags: vec![0; n],
            body: vec![0; n],
            surface: vec![f32::NAN; n],
            basins: vec![Basin::kept(Kind::Lake, 5.0, at(2, 2) as u32)],
            rivers: vec![River {
                id: 0,
                coarse: false,
                mouth: at(3, 2) as u32,
                end: End::Sea,
                cells: 2,
                wet_cells: 2,
                length_m: 2.0,
                drainage_km2: 20.0,
            }],
            band: vec![Band {
                cell: at(2, 3) as u32,
                owner: at(2, 2) as u32,
                depth: 1.0,
            }],
        };
        for x in 0..W {
            s.ground[at(4, x)] = -1.0;
            s.flags[at(4, x)] = SEA;
            s.dir[at(4, x)] = 0;
            s.surface[at(4, x)] = 0.0;
        }
        let lake = at(1, 1);
        (s.ground[lake], s.body[lake], s.surface[lake], s.dir[lake]) = (3.0, 1, 5.0, 8);
        for (y, g, w) in [(2, 4.0, 5.0), (3, 2.5, 3.0)] {
            let i = at(y, 2);
            (s.ground[i], s.surface[i], s.body[i]) = (g, w, 1);
            s.flags[i] |= RIVER | CENTRE;
        }
        // A channel cell beside the lake exit, at its surface.
        let i = at(2, 3);
        (s.ground[i], s.surface[i], s.body[i], s.flags[i]) = (4.0, 5.0, 1, RIVER);
        s
    }

    fn run(s: &Scene) -> Map<String, Value> {
        let acc = vec![1.0; s.ground.len()];
        let c = Context {
            dims: Dims { w: W, h: W },
            ground: &s.ground,
            dir: &s.dir,
            acc: &acc,
            flags: &s.flags,
            body: &s.body,
            surface: &s.surface,
            basins: &s.basins,
            rivers: &s.rivers,
            band: &s.band,
            cell_km2: 1.0,
            spacing_m: 1000.0,
            open_rows: [false, false],
            fixed_rows: 0,
            carve_floor: &[],
            held: [0, 0],
            halo: &[],
            seam: &SeamStep::none(),
        };
        checks(&c, 0).0
    }

    fn failed(s: &Scene) -> Vec<String> {
        run(s)
            .into_iter()
            .filter(|(_, v)| v.as_u64() != Some(0))
            .map(|(k, _)| k)
            .collect()
    }

    #[test]
    fn a_slot_or_step_at_the_seam_fails() {
        // 5 x 7, two taken rows, input flat at 10 m. Cut (2, 2) 8 m: a 1-row slot in the
        // first free row and a step across the seam; raise (4, 6) 61 m: over the raise limit.
        let (w, h) = (5, 7);
        let mut ground = vec![10.0f32; w * h];
        ground[2 * w + 2] = 2.0;
        ground[6 * w + 4] = 71.0;
        let input = vec![10.0f32; w];
        let surface = vec![f32::NAN; w * h];
        let (mut carve, mut slots) = (Carve::default(), Slots::new(w, 2));
        for y in 1..h {
            slots.add(y, &input, &ground, &surface);
            if y >= 2 {
                carve.add(y, &input, &ground[y * w..(y + 1) * w], 1.0);
            }
        }
        let mut flags = vec![0u8; w * h];
        flags[..2 * w].fill(FIXED);
        let (dir, acc) = (vec![2u8; w * h], vec![0f32; w * h]);
        let body = vec![0u32; w * h];
        let c = Context {
            dims: Dims { w, h },
            ground: &ground,
            dir: &dir,
            acc: &acc,
            flags: &flags,
            body: &body,
            surface: &surface,
            basins: &[],
            rivers: &[],
            band: &[],
            cell_km2: 1.0,
            spacing_m: 1.0,
            open_rows: [false, false],
            fixed_rows: 2,
            carve_floor: &[],
            held: [0, 0],
            halo: &[],
            seam: &SeamStep::none(),
        };
        let floor = vec![f32::NEG_INFINITY; w * h];
        let (m, _) = carving(&c, &floor, &[], 2, &|_| 0, &carve, &slots);
        for name in ["seam_slots", "seam_steps", "ground_raised_over_limit"] {
            assert_eq!(m[name], json!(1), "{name}: {m:?}");
        }
        assert_eq!(m["ground_below_carve_floor"], json!(0));
    }

    #[test]
    fn every_check_can_fail() {
        assert!(failed(&scene()).is_empty(), "{:?}", failed(&scene()));
        let broken: [(&str, Mutation); 16] = [
            ("guard_land_cells_not_draining_to_an_outlet", |s| {
                s.dir[at(2, 1)] = 0
            }),
            ("guard_lakes_with_several_levels", |s| {
                let i = at(1, 2);
                (s.ground[i], s.body[i], s.surface[i], s.dir[i]) = (3.0, 1, 5.5, 2);
                s.flags[i] = 0;
            }),
            ("land_cells_draining_uphill", |s| s.ground[at(3, 1)] = 11.0),
            ("lakes_in_several_pieces", |s| {
                let i = at(1, 4);
                (s.ground[i], s.body[i], s.surface[i]) = (3.0, 1, 5.0);
            }),
            ("lake_cells_not_below_level", |s| s.ground[at(1, 1)] = 6.0),
            ("lake_exits_off_level", |s| s.surface[at(2, 2)] = 5.5),
            ("river_cells_below_adjacent_lake", |s| {
                s.surface[at(2, 2)] = 4.5
            }),
            ("wet_cells_with_surface_not_above_ground", |s| {
                s.surface[at(2, 2)] = 3.9
            }),
            ("river_cells_rising_downstream", |s| {
                s.surface[at(3, 2)] = 5.1
            }),
            ("guard_river_beds_rising_downstream", |s| {
                s.ground[at(3, 2)] = 4.5
            }),
            // Ahead of the river, but below every water surface beside it.
            ("dry_cells_in_holes", |s| s.ground[at(3, 3)] = -1.0),
            ("rivers_ending_on_dry_land", |s| s.rivers[0].end = End::Land),
            ("dry_cells_below_adjacent_lake", |s| {
                s.ground[at(0, 1)] = 4.0
            }),
            ("dry_cells_below_adjacent_sea", |s| {
                s.ground[at(3, 0)] = -0.5
            }),
            ("dry_cells_below_adjacent_river", |s| {
                s.ground[at(3, 3)] = 2.0
            }),
            // Beside the channel cell, below its surface.
            ("dry_cells_below_adjacent_river", |s| {
                s.ground[at(2, 4)] = 4.5
            }),
        ];
        for (name, mutate) in broken {
            let mut s = scene();
            mutate(&mut s);
            assert!(
                failed(&s).contains(&name.to_string()),
                "{name}: {:?}",
                failed(&s)
            );
        }
    }
}
