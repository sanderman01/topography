//! Receivers on dry land: each cell takes the neighbour on its fall line (Horn's 3 x 3
//! gradient), dithered between the two directions that bracket the gradient by an R2
//! low-discrepancy threshold of its world position, when that neighbour is strictly lower;
//! else its steepest strictly lower neighbour; else none (a pit). Plain steepest descent
//! draws straight axis and 45° rivers on smooth slopes (v2's `flood::fall_line`).

use super::geom::{OFFSETS, Rect, World, step_len};

/// Directions at 0°, 45°, … from east turning south (as `OFFSETS` indices).
const AROUND: [usize; 8] = [3, 7, 1, 6, 2, 4, 0, 5];

/// Direction bytes (0 none) of the cells of `core`; `z` covers `win`, which holds `core`
/// grown by at least one cell. World-edge cells get none: water leaves the map there.
pub(crate) fn receivers(world: World, win: Rect, z: &[f32], core: Rect) -> Vec<u8> {
    let mut out = vec![0u8; core.cells()];
    for (k, o) in out.iter_mut().enumerate() {
        let (x, y) = core.world_xy(k);
        if world.is_edge(x, y) {
            continue;
        }
        let at = |dy: i64, dx: i64| {
            let (nx, ny) = ((x as i64 + dx) as u64, (y as i64 + dy) as u64);
            z[win.local(nx, ny).expect("receivers need a one-cell halo")]
        };
        let zc = at(0, 0);
        let mut best: Option<(f32, u8)> = None;
        for (j, &(dy, dx)) in OFFSETS.iter().enumerate() {
            let drop = zc - at(dy, dx);
            if drop > 0.0 {
                let s = drop / step_len(j as u8 + 1) as f32;
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, j as u8 + 1));
                }
            }
        }
        let Some((_, steepest)) = best else { continue };
        *o = steepest;
        let Some(turns) = fall_turns(|dy, dx| f64::from(at(dy, dx))) else {
            continue;
        };
        let pick = fall_pick(x, y, turns, |_| true).expect("every direction allowed");
        let (dy, dx) = OFFSETS[pick];
        if at(dy, dx) < zc {
            *o = pick as u8 + 1;
        }
    }
    out
}

/// Receivers on `z` that keep `prior`'s (the ground before channels were carved) wherever that
/// neighbour is not higher on `z` (ties keep it: a bed carved flat to a pond's level must not
/// send the river off its line); elsewhere `z`'s own. Carving can tip a dithered Horn pick off
/// the carved river (scale diagnosis 3); this keeps the routing on it. No cycle: kept edges
/// never rise, new picks strictly fall, and equal-height kept edges come from the acyclic prior
/// routing. `prior` covers `win` like `z`.
pub(crate) fn kept_receivers(
    world: World,
    win: Rect,
    z: &[f32],
    prior: &[f32],
    core: Rect,
) -> Vec<u8> {
    let old = receivers(world, win, prior, core);
    let mut out = receivers(world, win, z, core);
    for (k, (o, &d)) in out.iter_mut().zip(&old).enumerate() {
        if d == 0 {
            continue;
        }
        let (x, y) = core.world_xy(k);
        let (nx, ny) = world
            .neighbour(x, y, d as usize - 1)
            .expect("receiver inside");
        let at = |x: u64, y: u64| z[win.local(x, y).expect("receivers need a one-cell halo")];
        if at(nx, ny) <= at(x, y) {
            *o = d;
        }
    }
    out
}

/// Horn's fall line of the 3 x 3 window `f(dy, dx)`, in eighths of a turn from east turning
/// south; none on a flat.
pub(crate) fn fall_turns(f: impl Fn(i64, i64) -> f64) -> Option<f64> {
    // Horn: east and south rises; the water goes the other way.
    let east = (f(-1, 1) + 2.0 * f(0, 1) + f(1, 1)) - (f(-1, -1) + 2.0 * f(0, -1) + f(1, -1));
    let south = (f(1, -1) + 2.0 * f(1, 0) + f(1, 1)) - (f(-1, -1) + 2.0 * f(-1, 0) + f(-1, 1));
    if east == 0.0 && south == 0.0 {
        return None;
    }
    Some((-south).atan2(-east).rem_euclid(std::f64::consts::TAU) / std::f64::consts::FRAC_PI_4)
}

/// The direction (an `OFFSETS` index) on the fall line `turns` at (x, y): one of the two that
/// bracket it, dithered by an R2 low-discrepancy threshold of the world position (alike in
/// every tiling). If `ok` refuses that one, the allowed direction nearest the dithered line.
pub(crate) fn fall_pick(x: u64, y: u64, turns: f64, ok: impl Fn(usize) -> bool) -> Option<usize> {
    let sector = turns.floor() as usize % 8;
    let t = (x as f64 * 0.754_877_666_2 + y as f64 * 0.569_840_290_9).rem_euclid(1.0);
    let first = if turns.fract() > t {
        (sector + 1) % 8
    } else {
        sector
    };
    if ok(AROUND[first]) {
        return Some(AROUND[first]);
    }
    // The dithered pick is the direction nearest this line.
    let line = turns + 0.5 - t;
    let off = |p: usize| {
        let d = (line - p as f64).rem_euclid(8.0);
        d.min(8.0 - d)
    };
    (0..8)
        .filter(|&p| ok(AROUND[p]))
        .min_by(|&a, &b| off(a).total_cmp(&off(b)).then(a.cmp(&b)))
        .map(|p| AROUND[p])
}

#[cfg(test)]
mod tests {
    use super::super::geom::tiles;
    use super::*;

    fn plane(w: usize, h: usize, deg: f32) -> Vec<f32> {
        let a = deg.to_radians();
        (0..w * h)
            .map(|i| 100.0 - (i % w) as f32 * a.cos() - (i / w) as f32 * a.sin())
            .collect()
    }

    #[test]
    fn paths_follow_the_fall_line_on_a_plane() {
        // A plane falling 20° south of east: paths mix E and SE steps, never a straight line.
        let world = World { w: 60, h: 60 };
        let all = Rect::whole(world);
        let z = plane(60, 60, 20.0);
        let dir = receivers(world, all, &z, all);
        let (mut x, mut y, mut steps) = (1i64, 10i64, 0);
        loop {
            let d = dir[(y * 60 + x) as usize];
            if d == 0 {
                break;
            }
            let (dy, dx) = OFFSETS[d as usize - 1];
            (x, y, steps) = (x + dx, y + dy, steps + 1);
        }
        let slope = (y - 10) as f32 / (x - 1) as f32;
        assert!(steps > 20);
        assert!(
            (slope - 20f32.to_radians().tan()).abs() < 0.1,
            "path slope {slope}"
        );
    }

    /// A cell on a slope falling W and N: its Horn pick on the ground before carving (W or NW)
    /// stays after a carve that turns the gradient, as long as it is not higher (a tie keeps
    /// it); once it is higher, the carved ground's own pick takes over.
    #[test]
    fn kept_receivers_hold_the_prior_pick_while_it_stays_lower() {
        let world = World { w: 5, h: 5 };
        let all = Rect::whole(world);
        let prior: Vec<f32> = (0..25)
            .map(|i| 10.0 + (i % 5) as f32 + 0.5 * (i / 5) as f32)
            .collect();
        let c = 12;
        let pick = receivers(world, all, &prior, all)[c];
        let (w, nw) = (2 + 1, 4 + 1);
        assert!(pick == w || pick == nw, "a W or NW pick: {pick}");
        // Lower the side away from the pick by 3 m: the fall line turns past the other one.
        let mut carved = prior.clone();
        let side: [usize; 3] = if pick == w { [6, 7, 8] } else { [16, 17, 18] };
        for k in side {
            carved[k] -= 3.0;
        }
        let fresh = receivers(world, all, &carved, all)[c];
        assert_ne!(fresh, pick, "carving flips the plain pick");
        assert_eq!(kept_receivers(world, all, &carved, &prior, all)[c], pick);
        // A bed carved flat to its prior receiver (fix round 3, a pond rim's level): kept.
        let (dy, dx) = OFFSETS[pick as usize - 1];
        let r = ((2 + dy) * 5 + 2 + dx) as usize;
        carved[c] = carved[r];
        assert_eq!(kept_receivers(world, all, &carved, &prior, all)[c], pick);
        // The prior receiver higher: the carved ground's pick.
        carved[c] = carved[r] - 0.1;
        assert_eq!(
            kept_receivers(world, all, &carved, &prior, all)[c],
            receivers(world, all, &carved, all)[c]
        );
    }

    #[test]
    fn receivers_are_strictly_lower_and_edges_have_none() {
        let world = World { w: 30, h: 30 };
        let all = Rect::whole(world);
        let z: Vec<f32> = (0..900)
            .map(|i| ((i * 7919) % 101) as f32 * 0.1 + (i % 30) as f32 * 0.05)
            .collect();
        let dir = receivers(world, all, &z, all);
        for k in 0..900 {
            let (x, y) = all.world_xy(k);
            if world.is_edge(x, y) {
                assert_eq!(dir[k], 0);
            } else if dir[k] != 0 {
                let (nx, ny) = world.neighbour(x, y, dir[k] as usize - 1).expect("inside");
                assert!(z[all.local(nx, ny).expect("in")] < z[k]);
            }
        }
    }

    #[test]
    fn tiles_pick_the_same_receivers_as_the_whole() {
        let world = World { w: 40, h: 40 };
        let all = Rect::whole(world);
        let z: Vec<f32> = (0..1600)
            .map(|i| {
                100.0 - (i % 40) as f32 * 0.9 - (i / 40) as f32 * 0.4 + ((i * 13) % 5) as f32 * 0.05
            })
            .collect();
        let whole = receivers(world, all, &z, all);
        for core in tiles(world, 16) {
            let win = core.grow(1, world);
            let zw: Vec<f32> = (0..win.cells())
                .map(|k| {
                    let (x, y) = win.world_xy(k);
                    z[all.local(x, y).expect("in")]
                })
                .collect();
            let part = receivers(world, win, &zw, core);
            for (k, &p) in part.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                assert_eq!(p, whole[all.local(x, y).expect("in")]);
            }
        }
    }

    /// Straight-run share of steepest-descent vs fall-line paths at E49249 N16381 (the
    /// straight river of the diagnosis): cargo test … receivers -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_straight_runs_at_e49249() {
        use super::super::probe;
        let Some((win, z)) = probe::window(49249, 16381, 1024) else {
            return;
        };
        let core = Rect {
            x0: win.x0 + 1,
            y0: win.y0 + 1,
            w: win.w - 2,
            h: win.h - 2,
        };
        let world = probe::WORLD;
        let fall = receivers(world, win, &z, core);
        // Steepest descent alone, for comparison; ties go to the first direction, as in
        // `receivers`.
        let steep: Vec<u8> = (0..core.cells())
            .map(|k| {
                let (x, y) = core.world_xy(k);
                let zc = z[win.local(x, y).expect("in")];
                let mut best: Option<(f32, u8)> = None;
                for j in 0..8 {
                    let (nx, ny) = world.neighbour(x, y, j).expect("inside");
                    let d = (zc - z[win.local(nx, ny).expect("in")]) / step_len(j as u8 + 1) as f32;
                    if d > 0.0 && best.is_none_or(|(b, _)| d > b) {
                        best = Some((d, j as u8 + 1));
                    }
                }
                best.map_or(0, |b| b.1)
            })
            .collect();
        // Thalweg cells: in-window accumulated area of at least 0.05 km² (1 m² cells).
        let thalweg = |dir: &[u8]| {
            let mut order: Vec<usize> = (0..core.cells()).collect();
            let zat = |k: usize| {
                let (x, y) = core.world_xy(k);
                z[win.local(x, y).expect("in")]
            };
            order.sort_by(|&a, &b| zat(b).total_cmp(&zat(a)));
            let mut area = vec![1u64; core.cells()];
            for k in order {
                if dir[k] == 0 {
                    continue;
                }
                let (x, y) = core.world_xy(k);
                let (dy, dx) = OFFSETS[dir[k] as usize - 1];
                if let Some(n) = core.local((x as i64 + dx) as u64, (y as i64 + dy) as u64) {
                    area[n] += area[k];
                }
            }
            let max = area.iter().copied().max().unwrap_or(0);
            (
                area.iter().map(|&a| a >= 50_000).collect::<Vec<bool>>(),
                max,
            )
        };
        let share = |dir: &[u8]| {
            // Thalweg cells in same-direction runs of 60 or more steps along their receivers.
            let (on, max) = thalweg(dir);
            let (mut straight, mut total) = (0usize, 0usize);
            for k in 0..core.cells() {
                if dir[k] == 0 || !on[k] {
                    continue;
                }
                total += 1;
                let (mut run, mut c) = (0, k);
                while run < 60 {
                    let (x, y) = core.world_xy(c);
                    let (dy, dx) = OFFSETS[dir[k] as usize - 1];
                    let Some(n) = core.local((x as i64 + dx) as u64, (y as i64 + dy) as u64) else {
                        break;
                    };
                    if dir[n] != dir[k] {
                        break;
                    }
                    (run, c) = (run + 1, n);
                }
                if run >= 60 {
                    straight += 1;
                }
            }
            (straight as f64 / total.max(1) as f64, total, max)
        };
        let ((s_share, s_n, s_max), (f_share, f_n, f_max)) = (share(&steep), share(&fall));
        eprintln!(
            "thalweg straight-run share: steepest {s_share:.3} of {s_n} (largest area {s_max} m²), \
             fall line {f_share:.3} of {f_n} (largest area {f_max} m²)"
        );
        probe::shade_png("t3-e49249-raw", &z, win.w, &vec![false; z.len()]);
    }
}
