//! Depressions of the filled surface, and which of them stay as lakes or dry basins.

use super::flood::{Dims, FIXED, SEA, receiver};
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
}
