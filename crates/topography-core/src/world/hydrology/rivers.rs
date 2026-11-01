//! Rivers: cells with at least a threshold discharge (rain-weighted drainage area) form the
//! centreline, one body per river system (all river cells upstream of one mouth). A channel
//! of width and depth from discharge is carved below a water surface that never rises
//! downstream and never stands above a dry bank beside or behind it.

use std::collections::HashMap;

use super::flood::{CENTRE, Dims, FIXED, RIVER, SEA, receiver};
use super::lakes::{Basin, Kind};

/// Judgment constants: width W = WIDTH_A · Q^0.5 and depth D = DEPTH_B · Q^0.4 (m), with Q
/// the discharge in km² at mean rain: 8 m by 1.3 m at 10 km², 33 m by 4 m at 178 km².
pub(super) const WIDTH_A: f32 = 2.5;
pub(super) const DEPTH_B: f32 = 0.5;
/// A cell beside the centreline joins the channel where its mean depth is this share of D.
const BAND_MIN_SHARE: f32 = 0.1;
/// Sea mouths: the bed reaches below 0 m over this many cells and continues into the seabed.
pub(super) const MOUTH_CELLS: usize = 4;
/// The previous band's ground may sit this far below a river surface beside it (f32 rounding).
const SEAM_BANK_TOLERANCE_M: f32 = super::report::BANK_TOLERANCE_M;
/// Judgment constant: a surface drop forced by a low bank tapers upstream by this slope.
pub(super) const GRADE_SLOPE: f32 = 0.05;
/// Shallowest centreline water, so the surface stays above the bed in f32.
const MIN_DEPTH_M: f32 = 0.01;

pub(super) fn width_m(q_km2: f32) -> f32 {
    WIDTH_A * q_km2.max(0.0).sqrt()
}

pub(super) fn depth_m(q_km2: f32) -> f32 {
    DEPTH_B * q_km2.max(0.0).powf(0.4)
}

/// Mean depth over a cell `l` wide whose centre lies `r` from the channel axis, for a
/// parabolic section `w` wide and `d` deep. Sub-cell channels give a shallower single cell.
pub(super) fn cell_depth(r: f32, w: f32, d: f32, l: f32) -> f32 {
    let (lo, hi) = ((r - l / 2.0).max(-w / 2.0), (r + l / 2.0).min(w / 2.0));
    if hi <= lo || w <= 0.0 {
        return 0.0;
    }
    let area = |x: f32| x - 4.0 * x * x * x / (3.0 * w * w);
    d * (area(hi) - area(lo)) / l
}

/// Where a river system ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    Sea,
    Lake(u32),
    /// A closed dry basin (sink): an endorheic river.
    Sink(u32),
    /// A dry basin flow passes through (glacier ice): the river continues past its exit.
    Dry(u32),
    Edge,
    /// A confluence with a river whose id was given (index + 1): a coarse river or one from
    /// the previous band.
    Joins(u32),
    /// Dry land: never expected (rivers grow downstream), reported as a failure.
    Land,
}

pub(super) struct River {
    /// Body id: given for coarse rivers and rivers from the previous band, else 0 until ids
    /// are assigned.
    pub id: u32,
    pub coarse: bool,
    pub mouth: u32,
    pub end: End,
    /// Centreline cells.
    pub cells: u32,
    /// Centreline and channel cells.
    pub wet_cells: u32,
    pub length_m: f64,
    pub drainage_km2: f32,
}

/// A channel cell beside the centreline: its centreline cell and water depth.
pub(super) struct Band {
    pub cell: u32,
    pub owner: u32,
    pub depth: f32,
}

/// The centreline cell a river cell belongs to (itself on the centreline, and for a channel
/// cell copied from the previous band).
pub(super) fn owner(flags: &[u8], band: &[Band], i: usize) -> Option<usize> {
    if flags[i] & CENTRE != 0 {
        Some(i)
    } else if flags[i] & RIVER != 0 {
        Some(
            band.binary_search_by_key(&(i as u32), |b| b.cell)
                .map_or(i, |k| band[k].owner as usize),
        )
    } else {
        None
    }
}

/// Rivers with given ids and where they run, for `extract` (none at the trunk level).
#[derive(Default)]
pub(super) struct Given {
    /// Coarse rivers, then rivers from the previous band; cells copied from that band are
    /// already flagged and labelled (index + 1).
    pub rivers: Vec<River>,
    /// Centreline cells of a given river (index + 1): the coarse rivers' fine paths.
    pub cells: Vec<(u32, u32)>,
    /// Cells where a river from the previous band enters: it continues down the routing
    /// while the discharge stays above the threshold.
    pub follow: Vec<(u32, u32)>,
    /// Highest surface per centreline cell (the coarse surface along coarse paths).
    pub cap: HashMap<u32, f32>,
}

/// Channel figures.
#[derive(Default)]
pub(super) struct Channels {
    pub band_cells: u64,
    /// Ground lowered by channel carving (cells, deepest, volume).
    pub cut_cells: u64,
    /// Cut more than 10 m.
    pub deep_cells: u64,
    pub max_cut_m: f32,
    pub max_cut_at: usize,
    pub volume_m3: f64,
    /// Total carving cut per cell (m).
    pub cuts: HashMap<u32, f32>,
    /// Largest drop of a centreline surface below its ground forced by a low bank.
    pub max_bank_drop_m: f32,
    pub sea_cells_lowered: u64,
    pub widest_m: f32,
    pub deepest_m: f32,
    /// New streams dropped because they ran onto dry ground of the previous band.
    pub streams_pruned: u64,
    /// Centreline cells raised to an adjacent lake's level, or behind one (sorted).
    pub raised: Vec<u32>,
}

/// Mark the centreline (flags `RIVER | CENTRE`) and channel cells (`RIVER`), store river ids
/// (index + 1) in `body`, set water surfaces and carve the beds into `ground`.
#[allow(clippy::too_many_arguments)]
pub(super) fn extract(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    acc: &[f32],
    flags: &mut [u8],
    body: &mut [u32],
    basins: &[Basin],
    river_km2: f32,
    spacing_m: f64,
    surface: &mut [f32],
    given: Given,
    carve_floor: &[f32],
) -> (Vec<River>, Vec<Band>, Channels) {
    let n = ground.len();
    let l = spacing_m as f32;
    let free = |flags: &[u8], body: &[u32], i: usize| {
        flags[i] & (SEA | RIVER | FIXED) == 0 && body[i] == 0
    };
    // Coarse paths also run over dry basin floors (at 1 m a glacier is a field of small ice
    // basins), not through lakes or endorheic sinks.
    let dry_floor = |flags: &[u8], body: &[u32], i: usize| {
        flags[i] & (SEA | RIVER | FIXED) == 0
            && body[i] != 0
            && basins[body[i] as usize - 1].kind == Kind::Dry
    };
    for &(i, r) in &given.cells {
        let i = i as usize;
        if free(flags, body, i) || dry_floor(flags, body, i) {
            flags[i] |= RIVER | CENTRE;
            body[i] = r;
        }
    }
    // A coarse path ending on dry land (its coarse dry basin has no hollow here) continues
    // down the routing to water, a basin, another river or the edge.
    for &(i, r) in &given.cells {
        let mut c = receiver(dims, dir, i as usize);
        while let Some(j) = c.filter(|&j| free(flags, body, j)) {
            flags[j] |= RIVER | CENTRE;
            body[j] = r;
            c = receiver(dims, dir, j);
        }
    }
    for &(i, r) in &given.follow {
        let mut c = Some(i as usize);
        while let Some(i) = c.filter(|&i| free(flags, body, i) && acc[i] >= river_km2) {
            flags[i] |= RIVER | CENTRE;
            body[i] = r;
            c = receiver(dims, dir, i);
        }
    }
    for i in 0..n {
        if free(flags, body, i) && acc[i] >= river_km2 {
            flags[i] |= RIVER | CENTRE;
        }
    }
    let centre = |flags: &[u8], i: usize| flags[i] & CENTRE != 0;
    let fixed = |flags: &[u8], i: usize| flags[i] & FIXED != 0;
    let lake = |flags: &[u8], body: &[u32], i: usize| {
        flags[i] & (SEA | RIVER) == 0
            && body[i] != 0
            && basins[body[i] as usize - 1].kind == Kind::Lake
    };
    let end_at = |flags: &[u8], body: &[u32], r: Option<usize>| match r {
        None => End::Edge,
        Some(r) if flags[r] & RIVER != 0 => End::Joins(body[r]),
        Some(r) if flags[r] & SEA != 0 => End::Sea,
        Some(r) if lake(flags, body, r) => End::Lake(body[r]),
        Some(r) if body[r] != 0 && basins[body[r] as usize - 1].kind == Kind::Sink => {
            End::Sink(body[r])
        }
        Some(r) if body[r] != 0 => End::Dry(body[r]),
        Some(_) => End::Land,
    };

    // Ids: walk down to a river cell with an id or the river's end, then fill in from below.
    // A river reaching a given one is a tributary with its own id.
    let given_count = given.rivers.len() as u32;
    let mut rivers: Vec<River> = given.rivers;
    let mut chain: Vec<u32> = Vec::new();
    for i in 0..n {
        if !centre(flags, i) || body[i] != 0 {
            continue;
        }
        chain.clear();
        let mut c = i;
        loop {
            chain.push(c as u32);
            match receiver(dims, dir, c) {
                Some(r) if centre(flags, r) && body[r] == 0 => c = r,
                _ => break,
            }
        }
        for &x in chain.iter().rev() {
            let x = x as usize;
            let id = match receiver(dims, dir, x) {
                Some(r) if centre(flags, r) && body[r] > given_count => body[r],
                r => {
                    rivers.push(River {
                        id: 0,
                        coarse: false,
                        mouth: x as u32,
                        end: end_at(flags, body, r),
                        cells: 0,
                        wet_cells: 0,
                        length_m: 0.0,
                        drainage_km2: acc[x],
                    });
                    rivers.len() as u32
                }
            };
            body[x] = id;
        }
    }
    // Given rivers end where their cells here leave them (the largest discharge such cell).
    let mut mouth: Vec<Option<usize>> = vec![None; given_count as usize];
    for i in 0..n {
        let k = body[i];
        if !centre(flags, i) || fixed(flags, i) || k == 0 || k > given_count {
            continue;
        }
        let out = receiver(dims, dir, i).is_none_or(|r| !centre(flags, r) || body[r] != k);
        let m = &mut mouth[k as usize - 1];
        if out && m.is_none_or(|m| acc[i] > acc[m]) {
            *m = Some(i);
        }
    }
    for (k, m) in mouth.iter().enumerate() {
        if let Some(m) = *m {
            let r = &mut rivers[k];
            (r.mouth, r.drainage_km2) = (m as u32, acc[m]);
            r.end = end_at(flags, body, receiver(dims, dir, m));
        }
    }
    // A new stream that runs onto dry ground copied from the previous band (which did not
    // see enough discharge for it there) is dropped: it would end on dry land. So is one
    // joining that band's river beside its dry ground lower than the river's surface: its
    // water would stand above ground that cannot be raised.
    let below_taken_bank = |m: usize, x: usize| {
        let s = surface[x];
        s.is_finite()
            && dims.neighbours(m).any(|(_, y)| {
                fixed(flags, y)
                    && flags[y] & (SEA | RIVER) == 0
                    && !lake(flags, body, y)
                    && ground[y] + SEAM_BANK_TOLERANCE_M < s
            })
    };
    let mut pruned = vec![false; rivers.len()];
    for (k, r) in rivers.iter().enumerate().skip(given_count as usize) {
        let m = r.mouth as usize;
        let into_taken = receiver(dims, dir, m).filter(|&x| fixed(flags, x));
        pruned[k] = into_taken.is_some_and(|x| {
            r.end == End::Land || (flags[x] & RIVER != 0 && below_taken_bank(m, x))
        });
    }
    for i in 0..n {
        if centre(flags, i) && !fixed(flags, i) && pruned[body[i] as usize - 1] {
            flags[i] &= !(RIVER | CENTRE);
            body[i] = 0;
        }
    }
    let streams_pruned = pruned.iter().filter(|&&p| p).count() as u64;
    for i in 0..n {
        if centre(flags, i) && !fixed(flags, i) {
            let river = &mut rivers[body[i] as usize - 1];
            river.cells += 1;
            river.wet_cells += 1;
            river.length_m += Dims::step(dir[i]) * spacing_m;
        }
    }

    let mut band = channel_band(dims, ground, acc, flags, body, basins, l);
    for b in &band {
        let (i, o) = (b.cell as usize, b.owner as usize);
        flags[i] |= RIVER;
        body[i] = body[o];
        rivers[body[o] as usize - 1].wet_cells += 1;
    }
    let mut ch = Channels {
        band_cells: band.len() as u64,
        streams_pruned,
        ..Channels::default()
    };

    // Surfaces at the centreline: its ground, at most every dry bank beside or behind any of
    // its channel cells, then at most every surface upstream (upstream first, by donors).
    // Cells copied from the previous band keep their surface.
    let sills: HashMap<u32, f32> = (0..n)
        .filter(|&i| centre(flags, i) && !fixed(flags, i))
        .filter_map(|i| sill_level(dims, dir, flags, body, basins, i).map(|s| (i as u32, s)))
        .collect();
    for i in 0..n {
        if centre(flags, i) && !fixed(flags, i) {
            let s = sills.get(&(i as u32)).copied().unwrap_or(ground[i]);
            surface[i] = given.cap.get(&(i as u32)).map_or(s, |&c| s.min(c));
        }
    }
    let wet_cells = (0..n)
        .filter(|&i| centre(flags, i))
        .map(|i| (i, i))
        .chain(band.iter().map(|b| (b.cell as usize, b.owner as usize)));
    for (j, o) in wet_cells.collect::<Vec<_>>() {
        if sills.contains_key(&(o as u32)) || fixed(flags, o) {
            continue;
        }
        let entered = entered_basin(dims, dir, flags, body, o);
        for (_, k) in dims.neighbours(j) {
            if is_bank(dims, dir, flags, body, basins, entered, o, j, k) {
                surface[o] = surface[o].min(ground[k]);
            }
        }
    }
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i).filter(|&r| centre(flags, i) && centre(flags, r)) {
            donors[r] += 1;
        }
    }
    for i in 0..n {
        let mut c = i;
        while centre(flags, c) && donors[c] == 0 {
            donors[c] = u8::MAX;
            match receiver(dims, dir, c).filter(|&r| centre(flags, r)) {
                Some(r) => {
                    if !sills.contains_key(&(r as u32)) && !fixed(flags, r) {
                        surface[r] = surface[r].min(surface[c]);
                    }
                    donors[r] -= 1;
                    c = r;
                }
                None => break,
            }
        }
    }
    drop(donors);

    // Grade: where a low bank pulls the surface below the ground, the drop tapers upstream
    // by at most GRADE_SLOPE, so the bed ramps down over many cells instead of a notch.
    let mut order: Vec<u32> = (0..n)
        .filter(|&i| {
            centre(flags, i)
                && !fixed(flags, i)
                && receiver(dims, dir, i).is_none_or(|r| !centre(flags, r) || fixed(flags, r))
        })
        .map(|i| i as u32)
        .collect();
    let mut k = 0;
    while k < order.len() {
        let c = order[k] as usize;
        k += 1;
        ch.max_bank_drop_m = ch.max_bank_drop_m.max(ground[c] - surface[c]);
        for (_, d) in dims.neighbours(c) {
            if centre(flags, d) && !fixed(flags, d) && receiver(dims, dir, d) == Some(c) {
                order.push(d as u32);
                if !sills.contains_key(&(d as u32)) {
                    let taper = GRADE_SLOPE * (Dims::step(dir[d]) * spacing_m) as f32;
                    let drop = ground[c] - surface[c] - taper;
                    surface[d] = surface[d].min(ground[d] - drop);
                }
            }
        }
    }
    drop(order);

    // Beside a lake a river never stands below it: the surface rises to the level there (an
    // outflow along the shore stays at it). Nor below the water it flows into (a sill, a
    // fixed cell, a raised one). Lower surfaces upstream rise too (a backwater). Dry banks
    // below a raised surface are raised later, as lips are.
    let lake_beside = |flags: &[u8], body: &[u32], i: usize| {
        dims.neighbours(i)
            .filter(|&(_, j)| lake(flags, body, j))
            .map(|(_, j)| basins[body[j] as usize - 1].level)
            .fold(f32::NEG_INFINITY, f32::max)
    };
    // The previous band's dry ground beside a cell (it cannot be raised).
    let taken_bank = |flags: &[u8], body: &[u32], ground: &[f32], i: usize| {
        dims.neighbours(i)
            .filter(|&(_, k)| {
                fixed(flags, k) && flags[k] & (SEA | RIVER) == 0 && !lake(flags, body, k)
            })
            .map(|(_, k)| ground[k])
            .fold(f32::INFINITY, f32::min)
    };
    // How high a raise may lift a cell's water: that ground, and a coarse path's cap.
    let ceiling = |flags: &[u8], body: &[u32], ground: &[f32], i: usize| {
        let cap = given.cap.get(&(i as u32)).copied().unwrap_or(f32::INFINITY);
        taken_bank(flags, body, ground, i).min(cap)
    };
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        if centre(flags, i) && !fixed(flags, i) {
            let beside = lake_beside(flags, body, i);
            let below = receiver(dims, dir, i)
                .map(|r| if flags[r] & SEA != 0 { 0.0 } else { surface[r] })
                .filter(|s| !s.is_nan())
                .unwrap_or(f32::NEG_INFINITY);
            // Nor below its carve floor: the bed stops there, so the water stands on it (a
            // backwater upstream, banks raised later) instead of in a trench below it; but
            // not above the previous band's dry ground beside it (that cannot be raised) nor
            // a coarse path's cap. A lake exit stays at the lake's level.
            let floor = carve_floor
                .get(i)
                .filter(|_| !sills.contains_key(&(i as u32)))
                .map_or(f32::NEG_INFINITY, |&f| {
                    (f + MIN_DEPTH_M).min(ceiling(flags, body, ground, i))
                });
            let level = beside.max(below).max(floor);
            if surface[i] < level {
                surface[i] = level;
                ch.raised.push(i as u32);
                stack.push(i);
            }
        }
    }
    while let Some(c) = stack.pop() {
        for (_, d) in dims.neighbours(c) {
            if centre(flags, d)
                && !fixed(flags, d)
                && receiver(dims, dir, d) == Some(c)
                && surface[d] < surface[c]
            {
                surface[d] = surface[c];
                ch.raised.push(d as u32);
                stack.push(d);
            }
        }
    }
    // A backwater reaching beside the previous band's lower dry ground (or a coarse path's cap)
    // stops at it (not below a lake beside, nor below water it flows into that stays: the sea,
    // a fixed cell, a sill), and the water downstream stands no higher (its beds go just
    // below it).
    let held_below = |flags: &[u8], surface: &[f32], i: usize| {
        receiver(dims, dir, i)
            .filter(|&r| {
                flags[r] & SEA != 0
                    || (centre(flags, r) && (fixed(flags, r) || sills.contains_key(&(r as u32))))
            })
            .map(|r| if flags[r] & SEA != 0 { 0.0 } else { surface[r] })
            .filter(|s| !s.is_nan())
            .unwrap_or(f32::NEG_INFINITY)
    };
    for &i in &ch.raised {
        let i = i as usize;
        let cap = ceiling(flags, body, ground, i)
            .max(lake_beside(flags, body, i))
            .max(held_below(flags, surface, i));
        if surface[i] > cap && !sills.contains_key(&(i as u32)) {
            surface[i] = cap;
            stack.push(i);
        }
    }
    while let Some(c) = stack.pop() {
        if let Some(r) = receiver(dims, dir, c)
            .filter(|&r| centre(flags, r) && !fixed(flags, r) && !sills.contains_key(&(r as u32)))
        {
            let to = surface[c].max(held_below(flags, surface, r));
            if surface[r] > to {
                surface[r] = to;
                stack.push(r);
            }
        }
    }
    ch.raised.sort_unstable();
    ch.raised.dedup();

    // Gorge rule against the final surface: channel cells more than D above it stay dry
    // (they are above every surface beside them, so they are valid banks).
    // Nor beside a previous band's dry bank below the surface (that ground cannot be raised):
    // the cell becomes a bank itself, raised with the owner's other banks.
    let below_fixed_bank = |j: usize, o: usize| {
        let entered = entered_basin(dims, dir, flags, body, o);
        dims.neighbours(j).any(|(_, k)| {
            fixed(flags, k)
                && ground[k] + SEAM_BANK_TOLERANCE_M < surface[o]
                && is_bank(dims, dir, flags, body, basins, entered, o, j, k)
        })
    };
    let mut keep: Vec<bool> = Vec::with_capacity(band.len());
    for b in &band {
        let (j, o) = (b.cell as usize, b.owner as usize);
        let gorge = ground[j] > surface[o] + depth_m(acc[o])
            && !dims
                .neighbours(j)
                .any(|(_, k)| owner(flags, &band, k).is_some_and(|o2| ground[j] < surface[o2]));
        let seam = !fixed(flags, j) && below_fixed_bank(j, o);
        if seam {
            ch.raised.push(o as u32);
        }
        // Nor where its bed could not reach below the water (the carve floor).
        let floored = carve_floor
            .get(j)
            .is_some_and(|&f| f > surface[o] - MIN_DEPTH_M);
        keep.push(!gorge && !seam && !floored);
    }
    ch.raised.sort_unstable();
    ch.raised.dedup();
    let mut kept = keep.iter();
    band.retain(|b| {
        let ok = *kept.next().unwrap_or(&true);
        if !ok {
            let j = b.cell as usize;
            flags[j] &= !RIVER;
            rivers[body[j] as usize - 1].wet_cells -= 1;
            body[j] = 0;
        }
        ok
    });
    ch.band_cells = band.len() as u64;

    // Beds: the centreline below its surface by the cell's mean depth, reaching below 0 m
    // over the last cells to the sea.
    // A channel bed not below the carve floor (`band::ground_floor`; empty: none): the water
    // was raised to stand on it. Only beside the previous band's lower dry ground may the
    // water `s` stand lower (the bed just below it; `river_beds_below_carve_floor` bounds that).
    let floored = |i: usize, bed: f32, s: f32| {
        carve_floor
            .get(i)
            .map_or(bed, |&f| bed.max(f.min(s - MIN_DEPTH_M)))
    };
    let cut = |ground: &mut [f32], i: usize, bed: f32, ch: &mut Channels| {
        if bed < ground[i] {
            ch.cut_cells += 1;
            ch.deep_cells += u64::from(ground[i] - bed > 10.0);
            if ground[i] - bed > ch.max_cut_m {
                (ch.max_cut_m, ch.max_cut_at) = (ground[i] - bed, i);
            }
            ch.volume_m3 += (ground[i] - bed) as f64 * spacing_m * spacing_m;
            *ch.cuts.entry(i as u32).or_insert(0.0) += ground[i] - bed;
            ground[i] = bed;
        }
    };
    // Upstream of the previous band's channel, beds stay at or above its bed.
    let mut floor: HashMap<u32, f32> = HashMap::new();
    let mut stack: Vec<(usize, f32)> = (0..n)
        .filter(|&i| centre(flags, i) && !fixed(flags, i))
        .filter_map(|i| {
            receiver(dims, dir, i)
                .filter(|&r| fixed(flags, r) && centre(flags, r))
                .map(|r| (i, ground[r]))
        })
        .collect();
    while let Some((c, f)) = stack.pop() {
        let e = floor.entry(c as u32).or_insert(f32::NEG_INFINITY);
        if f <= *e {
            continue;
        }
        *e = f;
        for (_, d) in dims.neighbours(c) {
            if centre(flags, d) && !fixed(flags, d) && receiver(dims, dir, d) == Some(c) {
                stack.push((d, f));
            }
        }
    }
    for i in 0..n {
        if !centre(flags, i) || fixed(flags, i) {
            continue;
        }
        let (q, s) = (acc[i], surface[i]);
        let (w, d) = (width_m(q), depth_m(q));
        ch.widest_m = ch.widest_m.max(w);
        ch.deepest_m = ch.deepest_m.max(d);
        let mut bed = s - cell_depth(0.0, w, d, l).max(MIN_DEPTH_M);
        // Only near sea level: rivers off a cliff coast fall into the sea.
        if let Some(k) = cells_to_sea(dims, dir, flags, i).filter(|_| s <= MOUTH_CELLS as f32 * d) {
            bed = bed.min(-d * (MOUTH_CELLS - k) as f32 / MOUTH_CELLS as f32);
        }
        if let Some(&f) = floor.get(&(i as u32)) {
            bed = bed.max(f);
        }
        cut(ground, i, floored(i, bed, s), &mut ch);
    }
    // Beds never rise downstream (a raised surface leaves an older, deeper bed upstream):
    // each centreline bed at most every bed upstream, upstream first.
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i).filter(|&r| centre(flags, i) && centre(flags, r)) {
            donors[r] += 1;
        }
    }
    for i in 0..n {
        let mut c = i;
        while centre(flags, c) && donors[c] == 0 {
            donors[c] = u8::MAX;
            match receiver(dims, dir, c).filter(|&r| centre(flags, r)) {
                Some(r) => {
                    if !fixed(flags, r) && !fixed(flags, c) {
                        // Not below the carve floor: a pool under water instead.
                        let to = floored(r, ground[c], surface[r]);
                        cut(ground, r, to, &mut ch);
                    }
                    donors[r] -= 1;
                    c = r;
                }
                None => break,
            }
        }
    }
    drop(donors);
    for b in &band {
        let (j, o) = (b.cell as usize, b.owner as usize);
        surface[j] = surface[o];
        cut(
            ground,
            j,
            floored(j, surface[o] - b.depth, surface[o]),
            &mut ch,
        );
    }
    // The channel continues into the seabed past each sea mouth.
    for i in 0..n {
        if !centre(flags, i) || fixed(flags, i) || cells_to_sea(dims, dir, flags, i) != Some(0) {
            continue;
        }
        let (dy, dx) = Dims::offset(dir[i]);
        let step = depth_m(acc[i]) / MOUTH_CELLS as f32;
        let (mut y, mut x) = ((i / dims.w) as isize, (i % dims.w) as isize);
        for k in 1..=MOUTH_CELLS {
            (y, x) = (y + dy, x + dx);
            if y < 0 || x < 0 || y >= dims.h as isize || x >= dims.w as isize {
                break;
            }
            let j = y as usize * dims.w + x as usize;
            if flags[j] & SEA == 0 || fixed(flags, j) {
                break;
            }
            let bed = ground[i] - step * k as f32;
            if bed < ground[j] {
                ground[j] = bed;
                ch.sea_cells_lowered += 1;
            }
        }
    }
    (rivers, band, ch)
}

/// Dry ground `k` beside river cell `j` (centreline `o`) that the surface must not exceed:
/// not water, not the basin the river enters and not ahead along the flow.
#[allow(clippy::too_many_arguments)]
pub(super) fn is_bank(
    dims: Dims,
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    entered: u32,
    o: usize,
    j: usize,
    k: usize,
) -> bool {
    let lake = flags[k] & (SEA | RIVER) == 0
        && body[k] != 0
        && basins[body[k] as usize - 1].kind == Kind::Lake;
    flags[k] & (SEA | RIVER) == 0
        && !lake
        && (body[k] == 0 || body[k] != entered)
        && !dims.ahead(dir[o], j, k)
}

/// Cells beside the centreline within the channel width: each goes to its nearest
/// centreline cell (then the lower index). Not the sea, basins or next to a lake, and only
/// ground at most D above the centreline: in a gorge the channel stays narrower.
fn channel_band(
    dims: Dims,
    ground: &[f32],
    acc: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    l: f32,
) -> Vec<Band> {
    let lake = |i: usize| {
        flags[i] & (SEA | RIVER) == 0
            && body[i] != 0
            && basins[body[i] as usize - 1].kind == Kind::Lake
    };
    let mut best: HashMap<u32, (f32, u32, f32)> = HashMap::new();
    for c in 0..acc.len() {
        if flags[c] & CENTRE == 0 {
            continue;
        }
        let (w, d) = (width_m(acc[c]), depth_m(acc[c]));
        if w <= l {
            continue;
        }
        let reach = ((w / 2.0 + l / 2.0) / l).ceil() as isize;
        let (cy, cx) = ((c / dims.w) as isize, (c % dims.w) as isize);
        for y in (cy - reach).max(0)..=(cy + reach).min(dims.h as isize - 1) {
            for x in (cx - reach).max(0)..=(cx + reach).min(dims.w as isize - 1) {
                let j = y as usize * dims.w + x as usize;
                if j == c || flags[j] & (SEA | RIVER | FIXED) != 0 || body[j] != 0 {
                    continue;
                }
                let r = l * (((y - cy).pow(2) + (x - cx).pow(2)) as f32).sqrt();
                let depth = cell_depth(r, w, d, l);
                if depth < BAND_MIN_SHARE * d || ground[j] > ground[c] + d {
                    continue;
                }
                let e = best
                    .entry(j as u32)
                    .or_insert((f32::INFINITY, u32::MAX, 0.0));
                if (r, c as u32) < (e.0, e.1) {
                    *e = (r, c as u32, depth);
                }
            }
        }
    }
    let mut band: Vec<Band> = best
        .into_iter()
        .filter(|&(j, _)| !dims.neighbours(j as usize).any(|(_, k)| lake(k)))
        .map(|(cell, (_, owner, depth))| Band { cell, owner, depth })
        .collect();
    band.sort_unstable_by_key(|b| b.cell);
    band
}

/// Centreline cells from `i` to the sea along the receivers (0 at the mouth), if under
/// `MOUTH_CELLS`.
fn cells_to_sea(dims: Dims, dir: &[u8], flags: &[u8], i: usize) -> Option<usize> {
    let mut c = i;
    for k in 0..MOUTH_CELLS {
        match receiver(dims, dir, c) {
            Some(r) if flags[r] & SEA != 0 => return Some(k),
            Some(r) if flags[r] & CENTRE != 0 => c = r,
            _ => return None,
        }
    }
    None
}

/// Cells carrying a river (discharge from `river_km2`) whose water would have to climb: the
/// free dry cell they drain into has its carve floor (`floor`) above the water level arriving
/// there (the lowest ground, or lake level, on the way down), so only a trench below the floor
/// could carry it on. Not cells taken from the previous band, nor `keep` ones (coarse paths).
/// Each comes with its water level.
#[allow(clippy::too_many_arguments)]
pub(super) fn climbs(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    dir: &[u8],
    acc: &[f32],
    river_km2: f32,
    floor: &[f32],
    keep: impl Fn(usize) -> bool,
) -> Vec<(usize, f32)> {
    const DONE: u8 = u8::MAX;
    let n = ground.len();
    if floor.len() != n {
        return Vec::new();
    }
    let mut level: Vec<f32> = (0..n)
        .map(|i| match body[i] {
            b if b != 0 && flags[i] & (SEA | RIVER) == 0 => {
                basins[b as usize - 1].level.max(ground[i])
            }
            _ => ground[i],
        })
        .collect();
    let mut donors = vec![0u8; n];
    for i in 0..n {
        if let Some(r) = receiver(dims, dir, i) {
            donors[r] = donors[r].saturating_add(1);
        }
    }
    let mut out = Vec::new();
    for i in 0..n {
        let mut c = i;
        while donors[c] == 0 {
            donors[c] = DONE;
            let Some(r) = receiver(dims, dir, c) else {
                break;
            };
            let free = |j: usize| flags[j] & (FIXED | SEA) == 0 && !keep(j);
            if acc[c] >= river_km2
                && free(c)
                && free(r)
                && body[r] == 0
                && floor[r] > level[c] + 0.01
            {
                out.push((c, level[c]));
            } else {
                level[r] = level[r].min(level[c]);
            }
            donors[r] -= 1;
            c = r;
        }
    }
    out
}

/// For a river at `c` that would climb (`climbs`) with its water at `level` and no lower
/// neighbour to turn to: the way on whose water level stays highest, over free dry cells whose
/// carve floor is not above that level (a channel can be cut to it, no trench), to the first
/// cell from which the routing goes on without climbing. Returns the route from `c`, or None
/// (the river ends at `c`).
#[allow(clippy::too_many_arguments)]
pub(super) fn route_on(
    dims: Dims,
    ground: &[f32],
    flags: &[u8],
    body: &[u32],
    dir: &[u8],
    floor: &[f32],
    keep: impl Fn(usize) -> bool,
    c: usize,
    level: f32,
) -> Option<Vec<usize>> {
    // Cells a search may visit, and routing cells a check may follow, before giving up.
    const MAX_CELLS: usize = 1 << 16;
    const MAX_WALK: usize = 1 << 20;
    const TOLERANCE_M: f32 = 0.01;
    let free = |j: usize| flags[j] & (FIXED | SEA) == 0 && !keep(j) && body[j] == 0;
    // The routing from `x` on, as `climbs` follows it: no climb before water, a basin or an
    // end; and on through those to its end, never back onto the route (a loop).
    let clear = |route: &std::collections::HashSet<usize>, mut x: usize, mut lvl: f32| {
        let mut climbing = true;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..MAX_WALK {
            let Some(r) = receiver(dims, dir, x) else {
                return true;
            };
            // Back onto the route, or into a cycle already there.
            if route.contains(&r) || !seen.insert(r) {
                return false;
            }
            if !free(r) {
                climbing = false;
            }
            if climbing && floor[r] > lvl + TOLERANCE_M {
                return false;
            }
            lvl = lvl.min(ground[r]);
            x = r;
        }
        false
    };
    let mut from: HashMap<usize, usize> = HashMap::from([(c, c)]);
    let mut heap = std::collections::BinaryHeap::from([super::band::Widest(level, ground[c], c)]);
    while let Some(super::band::Widest(lvl, _, x)) = heap.pop() {
        if x != c {
            let mut route = vec![x];
            while route[route.len() - 1] != c {
                route.push(from[&route[route.len() - 1]]);
            }
            route.reverse();
            if clear(&route.iter().copied().collect(), x, lvl) {
                return Some(route);
            }
        }
        for (_, j) in dims.neighbours(x) {
            if from.contains_key(&j) || !free(j) || floor[j] > lvl + TOLERANCE_M {
                continue;
            }
            from.insert(j, x);
            heap.push(super::band::Widest(lvl.min(ground[j]), ground[j], j));
        }
        if from.len() > MAX_CELLS {
            break;
        }
    }
    None
}

/// A dry cell more than this below every adjacent water surface is a hole.
pub(super) const HOLE_M: f32 = 0.5;

/// For a dry cell with water beside it that lies more than `HOLE_M` below every adjacent water
/// surface (sea, lake or river; not inside the basin an adjacent river enters): the lowest
/// of those surfaces.
#[allow(clippy::too_many_arguments)]
pub(super) fn hole(
    dims: Dims,
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    band: &[Band],
    ground: &[f32],
    surface: &[f32],
    i: usize,
) -> Option<f32> {
    if flags[i] & (SEA | RIVER) != 0
        || (body[i] != 0 && basins[body[i] as usize - 1].kind == Kind::Lake)
    {
        return None;
    }
    let mut low = f32::INFINITY;
    for (_, j) in dims.neighbours(i) {
        let s = surface[j];
        if s.is_nan() {
            continue;
        }
        if body[i] != 0
            && owner(flags, band, j)
                .is_some_and(|o| entered_basin(dims, dir, flags, body, o) == body[i])
        {
            return None;
        }
        low = low.min(s);
    }
    (low.is_finite() && low > ground[i] + HOLE_M).then_some(low)
}

/// Raise each hole (dry basin cells too, as `hole` counts them) to its lowest adjacent water
/// surface, then every dry cell draining into a raised one that is now lower than it;
/// repeated while holes remain (a few passes). Cells from the previous band stay. Returns
/// what was raised.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_holes(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    band: &[Band],
    surface: &[f32],
    spacing_m: f64,
) -> Lips {
    let dry = |i: usize| {
        flags[i] & (SEA | RIVER | FIXED) == 0
            && (body[i] == 0 || basins[body[i] as usize - 1].kind != Kind::Lake)
    };
    let mut filled = Lips::default();
    let mut stack: Vec<u32> = Vec::new();
    for _ in 0..8 {
        let holes: Vec<(usize, f32)> = (0..ground.len())
            .filter(|&i| dry(i))
            .filter_map(|i| {
                hole(dims, dir, flags, body, basins, band, ground, surface, i).map(|s| (i, s))
            })
            .collect();
        if holes.is_empty() {
            break;
        }
        for (i, to) in holes {
            filled.raise(ground, i, to, spacing_m, &mut stack);
        }
        while let Some(r) = stack.pop() {
            let r = r as usize;
            for (_, d) in dims.neighbours(r) {
                if dry(d) && ground[d] < ground[r] && receiver(dims, dir, d) == Some(r) {
                    let to = ground[r];
                    filled.raise(ground, d, to, spacing_m, &mut stack);
                }
            }
        }
    }
    filled
}

/// Ground raised beside lake exits.
#[derive(Default)]
pub(super) struct Lips {
    pub cells: u64,
    pub max_m: f32,
    pub volume_m3: f64,
    /// Notch cells that join a lake (cell, basin index + 1) instead of being raised.
    pub joins: Vec<(u32, u32)>,
    /// Banks of this band's rivers still below the water when lips are raised (a step after
    /// the surfaces changed which cells are banks): raised by the final catch-all.
    pub late_banks: u64,
}

impl Lips {
    fn raise(
        &mut self,
        ground: &mut [f32],
        i: usize,
        to: f32,
        spacing_m: f64,
        stack: &mut Vec<u32>,
    ) {
        let by = to - ground[i];
        self.cells += 1;
        self.max_m = self.max_m.max(by);
        self.volume_m3 += by as f64 * spacing_m * spacing_m;
        ground[i] = to;
        stack.push(i as u32);
    }
}

/// Raise dry ground beside each lake exit's channel that is below the lake level up to it (a
/// rock sill), beside channels raised to a lake's level (`raised`) up to their surface, and
/// any dry cell beside a lake below its level up to it (a divide),
/// then every dry cell draining into a raised one that is now lower than it, so no ground
/// drains uphill. Lake cells are left as they are; dry basin cells are raised like dry land.
#[allow(clippy::too_many_arguments)]
pub(super) fn raise_lips(
    dims: Dims,
    ground: &mut [f32],
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    band: &[Band],
    surface: &[f32],
    raised: &[u32],
    spacing_m: f64,
) -> Lips {
    // Dry: not water (dry basin cells count).
    let dry = |i: usize| {
        flags[i] & (SEA | RIVER | FIXED) == 0
            && (body[i] == 0 || basins[body[i] as usize - 1].kind != Kind::Lake)
    };
    let mut lips = Lips::default();
    let mut stack: Vec<u32> = Vec::new();
    let mut raise = |ground: &mut [f32], i: usize, to: f32, stack: &mut Vec<u32>| {
        let by = to - ground[i];
        lips.cells += 1;
        lips.max_m = lips.max_m.max(by);
        lips.volume_m3 += by as f64 * spacing_m * spacing_m;
        ground[i] = to;
        stack.push(i as u32);
    };
    let wet = (0..ground.len())
        .filter(|&i| flags[i] & CENTRE != 0)
        .map(|i| (i, i))
        .chain(band.iter().map(|b| (b.cell as usize, b.owner as usize)));
    for (x, o) in wet {
        let Some(level) = sill_level(dims, dir, flags, body, basins, o).or_else(|| {
            raised
                .binary_search(&(o as u32))
                .is_ok()
                .then_some(surface[o])
        }) else {
            continue;
        };
        for (_, j) in dims.neighbours(x) {
            if dry(j) && ground[j] < level {
                raise(ground, j, level, &mut stack);
            }
        }
    }
    // The previous band's river cells hold their surface: dry cells of this band beside one
    // stand at least at it (that band's halo saw them as its banks).
    for x in (0..ground.len()).filter(|&x| flags[x] & (FIXED | RIVER) == FIXED | RIVER) {
        if surface[x].is_nan() {
            continue;
        }
        for (_, j) in dims.neighbours(x) {
            if dry(j) && ground[j] < surface[x] {
                raise(ground, j, surface[x], &mut stack);
            }
        }
    }
    // So do the banks of this band's channel cells that river owns (its surface is fixed).
    // Any other bank still below the water (a step after the surfaces were set, such as
    // widening lowering a river's mouth out of the basin it entered, changed the banks) is
    // raised to it too, beyond the report's tolerance.
    let mut late_banks = 0u64;
    let wet = (0..ground.len())
        .filter(|&i| flags[i] & (CENTRE | FIXED) == CENTRE)
        .map(|i| (i, i))
        .chain(band.iter().map(|b| (b.cell as usize, b.owner as usize)));
    for (x, o) in wet.collect::<Vec<_>>() {
        if flags[x] & FIXED != 0 || surface[o].is_nan() {
            continue;
        }
        let slack = if flags[o] & FIXED != 0 {
            0.0
        } else {
            SEAM_BANK_TOLERANCE_M
        };
        let entered = entered_basin(dims, dir, flags, body, o);
        for (_, j) in dims.neighbours(x) {
            if dry(j)
                && ground[j] + slack < surface[o]
                && is_bank(dims, dir, flags, body, basins, entered, o, x, j)
            {
                raise(ground, j, surface[o], &mut stack);
                late_banks += u64::from(slack > 0.0);
            }
        }
    }
    // A dry cell beside a lake below its level (left by carving next to a lake grown over
    // carved ground) becomes a divide at the level, or joins the lake when 6 or more of its
    // neighbours are that lake (a notch inside it).
    let mut joins: Vec<(u32, u32)> = Vec::new();
    for j in 0..ground.len() {
        if !dry(j) {
            continue;
        }
        let lake_at = |k: usize| {
            (flags[k] & (SEA | RIVER) == 0
                && body[k] != 0
                && basins[body[k] as usize - 1].kind == Kind::Lake)
                .then_some(body[k])
        };
        let Some(top) = dims
            .neighbours(j)
            .filter_map(|(_, k)| lake_at(k))
            .max_by(|&a, &b| {
                basins[a as usize - 1]
                    .level
                    .total_cmp(&basins[b as usize - 1].level)
            })
        else {
            continue;
        };
        let level = basins[top as usize - 1].level;
        if ground[j] < level {
            let inside = dims
                .neighbours(j)
                .filter(|&(_, k)| lake_at(k) == Some(top))
                .count();
            if body[j] == 0 && inside >= 6 {
                joins.push((j as u32, top));
                continue;
            }
            raise(ground, j, level, &mut stack);
        }
    }
    while let Some(r) = stack.pop() {
        let r = r as usize;
        for (_, d) in dims.neighbours(r) {
            if dry(d) && ground[d] < ground[r] && receiver(dims, dir, d) == Some(r) {
                let to = ground[r];
                raise(ground, d, to, &mut stack);
            }
        }
    }
    lips.joins = joins;
    lips.late_banks = late_banks;
    lips
}

/// A centreline cell some lake drains into (a lake exit): that lake's level.
pub(super) fn sill_level(
    dims: Dims,
    dir: &[u8],
    flags: &[u8],
    body: &[u32],
    basins: &[Basin],
    i: usize,
) -> Option<f32> {
    if flags[i] & CENTRE == 0 {
        return None;
    }
    dims.neighbours(i)
        .filter(|&(_, j)| {
            flags[j] & (SEA | RIVER) == 0
                && body[j] != 0
                && basins[body[j] as usize - 1].kind == Kind::Lake
                && receiver(dims, dir, j) == Some(i)
        })
        .map(|(_, j)| basins[body[j] as usize - 1].level)
        .reduce(f32::min)
}

/// The dry basin a river cell drains straight into, or 0. Its cells are not banks.
pub(super) fn entered_basin(dims: Dims, dir: &[u8], flags: &[u8], body: &[u32], i: usize) -> u32 {
    match receiver(dims, dir, i) {
        Some(r) if flags[r] & (SEA | RIVER) == 0 => body[r],
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::lakes::NO_EXIT;
    use super::*;

    #[test]
    fn a_climbing_river_goes_on_where_a_channel_can_be_cut() {
        // A 1 m flat on a 5 x 3 grid; the river's water at cell 6 stands at 0.5 m. Its
        // receiver 7 may not be cut below 1 m; cells 12, 13 and 14 may, and drain on east.
        let dims = Dims { w: 5, h: 3 };
        let ground = vec![1.0f32; 15];
        let mut floor = vec![1.0f32; 15];
        for i in [12, 13, 14] {
            floor[i] = 0.0;
        }
        let mut dir = vec![0u8; 15];
        dir[6] = dims.towards(6, 7);
        dir[12] = dims.towards(12, 13);
        dir[13] = dims.towards(13, 14);
        let (flags, body) = (vec![0u8; 15], vec![0u32; 15]);
        let route = route_on(
            dims,
            &ground,
            &flags,
            &body,
            &dir,
            &floor,
            |_| false,
            6,
            0.5,
        );
        assert_eq!(route, Some(vec![6, 12]));
        // Lake cell 5 drains into cell 6, and the lower shore cell 10 drains into the lake: a
        // route onto it would close a loop through the lake, so the route goes by cell 12.
        let mut body = body.clone();
        body[5] = 1;
        let mut ground = ground.clone();
        ground[10] = 0.9;
        floor[10] = 0.0;
        dir[5] = dims.towards(5, 6);
        dir[10] = dims.towards(10, 5);
        let route = route_on(
            dims,
            &ground,
            &flags,
            &body,
            &dir,
            &floor,
            |_| false,
            6,
            0.5,
        );
        assert_eq!(route, Some(vec![6, 12]));
        // No cell a channel can be cut in: the river ends there.
        let route = route_on(
            dims,
            &ground,
            &flags,
            &body,
            &dir,
            &[1.0; 15],
            |_| false,
            6,
            0.5,
        );
        assert_eq!(route, None);
    }

    #[test]
    fn a_coarse_path_crosses_a_dry_basin_and_continues_off_dry_land() {
        // Column 1 of a 3 x 6 grid drains south to the sea row. The coarse path holds rows 1
        // and 2; row 2 is a dry (ice) basin floor. It continues down to the sea.
        let dims = Dims { w: 3, h: 6 };
        let n = 18;
        let mut ground: Vec<f32> = (0..n).map(|i| 10.0 - (i / 3) as f32).collect();
        let mut flags = vec![0u8; n];
        for f in flags.iter_mut().skip(15) {
            *f = SEA;
        }
        let mut dir = vec![0u8; n];
        for (i, d) in dir.iter_mut().enumerate().take(15) {
            *d = 2;
            if i % 3 != 1 {
                *d = 0;
            }
        }
        let mut body = vec![0u32; n];
        body[7] = 1;
        let basins = vec![Basin {
            kind: Kind::Dry,
            level: 8.5,
            exit: NO_EXIT,
            filled_by_river: false,
            over_budget: false,
            ice: true,
            id: 50,
        }];
        let acc = vec![0.01f32; n];
        let river = River {
            id: 9,
            coarse: true,
            mouth: 0,
            end: End::Edge,
            cells: 0,
            wet_cells: 0,
            length_m: 0.0,
            drainage_km2: 0.0,
        };
        let given = Given {
            rivers: vec![river],
            cells: vec![(4, 1), (7, 1)],
            ..Default::default()
        };
        let mut surface = vec![f32::NAN; n];
        let (rivers, _, _) = extract(
            dims,
            &mut ground,
            &dir,
            &acc,
            &mut flags,
            &mut body,
            &basins,
            1.0,
            1.0,
            &mut surface,
            given,
            &[],
        );
        for i in [4, 7, 10, 13] {
            assert!(flags[i] & CENTRE != 0, "cell {i} is on the river");
            assert_eq!(body[i], 1);
        }
        assert!(rivers[0].end == End::Sea);
    }

    #[test]
    fn a_bed_never_goes_below_the_carve_floor() {
        // Column 1 of a 3 x 4 grid drains south to the sea row; a low bank at (0, 1) pulls
        // the river's water 5 m below its ground. The carve floor (8 m) holds the bed: the
        // water stands on it, upstream too, instead of in a trench.
        let dims = Dims { w: 3, h: 4 };
        let n = 12;
        let mut ground = vec![10.0f32; n];
        ground[3] = 5.0;
        let mut flags = vec![0u8; n];
        for i in 9..12 {
            (flags[i], ground[i]) = (SEA, -1.0);
        }
        let mut dir = vec![0u8; n];
        for i in [1, 4, 7] {
            dir[i] = 2;
        }
        let mut acc = vec![0.0f32; n];
        for i in [1, 4, 7] {
            acc[i] = 1.0;
        }
        let mut body = vec![0u32; n];
        let mut surface = vec![f32::NAN; n];
        let floor = vec![8.0f32; n];
        extract(
            dims,
            &mut ground,
            &dir,
            &acc,
            &mut flags,
            &mut body,
            &[],
            1.0,
            1.0,
            &mut surface,
            Given::default(),
            &floor,
        );
        for i in [1, 4, 7] {
            assert!(flags[i] & CENTRE != 0);
            assert!(ground[i] >= 8.0, "bed {i} at {}", ground[i]);
            assert!(surface[i] > ground[i], "water {i} above its bed");
        }
        assert!(surface[1] >= surface[4] && surface[4] >= surface[7]);
    }

    #[test]
    fn a_new_stream_joining_beside_low_taken_ground_is_dropped() {
        // Row 0 is the previous band's: its river at (2, 0) stands at 2 m beside its dry
        // ground at (1, 0), 1 m. A new stream down column 1 would join it at (1, 1).
        let dims = Dims { w: 4, h: 4 };
        let n = 16;
        let mut ground = vec![3.0f32; n];
        (ground[1], ground[2], ground[5], ground[9], ground[13]) = (1.0, 1.5, 2.2, 2.5, 3.0);
        let mut flags = vec![0u8; n];
        for f in flags.iter_mut().take(4) {
            *f = FIXED;
        }
        flags[2] |= RIVER | CENTRE;
        let mut body = vec![0u32; n];
        body[2] = 1;
        let mut dir = vec![0u8; n];
        (dir[13], dir[9], dir[5]) = (1, 1, 6);
        let mut acc = vec![0.0f32; n];
        (acc[13], acc[9], acc[5]) = (2.0, 2.0, 2.0);
        let mut surface = vec![f32::NAN; n];
        surface[2] = 2.0;
        let given = Given {
            rivers: vec![River {
                id: 7,
                coarse: false,
                mouth: 2,
                end: End::Edge,
                cells: 0,
                wet_cells: 0,
                length_m: 0.0,
                drainage_km2: 0.0,
            }],
            ..Default::default()
        };
        let (_, _, ch) = extract(
            dims,
            &mut ground,
            &dir,
            &acc,
            &mut flags,
            &mut body,
            &[],
            1.0,
            1.0,
            &mut surface,
            given,
            &[],
        );
        assert_eq!(ch.streams_pruned, 1);
        assert!([5, 9, 13].iter().all(|&i| flags[i] & RIVER == 0));
    }
}
