//! Breach or fill (Lindsay 2016's hybrid). A full depression is cut through when a graded
//! gorge drains it for less volume than it would hold: the gorge runs from the depression's
//! pit up its receivers to the spill and down the outlet's receivers to the first cell below
//! the pit; its bed falls linearly over at least GORGE_RATIO × its deepest cut, it is at least
//! 3 cells wide, and its deepest cut stays within the level's breach cap (at 1 m the 2 m
//! channel cap for every river, R9; the channels' raw − 2 m clamp keeps breach plus channel
//! within it; and only depressions a river flows into). Otherwise the depression keeps its
//! water. Gorges lie within `Params::breach_radius` cells of their spill, so every tiling
//! decides alike.

use super::GORGE_RATIO;
use super::balance::mm;
use super::geom::{Rect, World, step_len};
use super::hydraulic::Geometry;
use super::layers::Layer;
use super::pipeline::{Model, Setup, crop};
use super::rain::Q_PER_KM2;
use super::receivers::receivers;
use super::store::{Work, for_tiles};
use crate::acquisition::Result;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// The node's key (R28).
    pub key: u64,
    pub inside: u64,
    pub outside: u64,
    pub vfill_mm: i64,
    /// Gorge half-width in cells (at least 1: 3 cells wide).
    pub half: u64,
    pub cap: f32,
    /// Gorge cells lie within this many cells of `inside`.
    pub radius: u64,
}

/// Full nodes (never sea: sea is no node); at 1 m only those a river flows into. The cap is
/// the level's breach cap for every river (I1): at 1 m the channels' raw − 2 m clamp keeps
/// breach plus channel within 2 m.
pub(crate) fn candidates(s: &Setup, m: &Model, geo: &Geometry) -> Vec<Candidate> {
    let thr = (s.params.river_km2 * Q_PER_KM2) as u64;
    m.water
        .nodes
        .0
        .iter()
        .filter(|n| n.full)
        .filter_map(|n| {
            let inflow = m.flow.node_total.get(&n.key).copied().unwrap_or(0);
            let half = if s.params.channels {
                if inflow < thr {
                    return None;
                }
                (geo.width_m(inflow as f64 / Q_PER_KM2) / 2.0 / s.spacing_m)
                    .ceil()
                    .max(1.0) as u64
            } else {
                1
            };
            Some(Candidate {
                key: n.key,
                inside: n.outlet[0],
                outside: n.outlet[1],
                vfill_mm: n.vfill_mm,
                half,
                cap: s.params.breach_cap_m,
                radius: s.params.breach_radius,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub(crate) struct Gorge {
    pub key: u64,
    /// (world cell, new ground), sorted by cell.
    pub cut: Vec<(u64, f32)>,
    pub depth: f32,
    pub volume_mm: i64,
    pub length_m: f64,
}

/// The gorge draining `c`, or None (fill): it would leave the radius, cut deeper than the
/// cap, or remove more than the depression holds. `zw` is the ground over `win`, which must
/// hold `c.inside` grown by `c.radius + 1` (clipped to the world) for every tiling to agree.
pub(crate) fn gorge(
    world: World,
    win: Rect,
    zw: &[f32],
    c: &Candidate,
    spacing_m: f64,
) -> Option<Gorge> {
    let (ix, iy) = world.xy(c.inside);
    // Within the radius, with the 3 x 3 block receivers need inside the window.
    let near = |cell: u64| {
        let (x, y) = world.xy(cell);
        x.abs_diff(ix).max(y.abs_diff(iy)) <= c.radius
            && win.contains(x.saturating_sub(1), y.saturating_sub(1))
            && win.contains((x + 1).min(world.w - 1), (y + 1).min(world.h - 1))
    };
    let z = |cell: u64| {
        let (x, y) = world.xy(cell);
        zw[win.local(x, y).expect("near cells are in the window")]
    };
    let receiver = |cell: u64| -> Option<u64> {
        let (x, y) = world.xy(cell);
        let d = receivers(
            world,
            win,
            zw,
            Rect {
                x0: x,
                y0: y,
                w: 1,
                h: 1,
            },
        )[0];
        (d != 0).then(|| {
            let (nx, ny) = world.neighbour(x, y, d as usize - 1).expect("inside");
            world.index(nx, ny)
        })
    };
    if !near(c.inside) || !near(c.outside) {
        return None;
    }
    // Up: from the spill's inside cell down to its pit; the gorge starts at the pit.
    let mut path = vec![c.inside];
    while let Some(next) = receiver(*path.last()?) {
        if !near(next) {
            return None;
        }
        path.push(next);
    }
    path.reverse();
    let zp = z(path[0]);
    // Down: from the outlet to the first cell below the pit.
    let mut cur = c.outside;
    path.push(cur);
    while z(cur) >= zp {
        cur = receiver(cur).filter(|&n| near(n))?;
        path.push(cur);
    }
    // Lengthen downstream until the bed falls over GORGE_RATIO × the deepest cut.
    loop {
        let mut s = vec![0.0f64; path.len()];
        for i in 1..path.len() {
            let (a, b) = (world.xy(path[i - 1]), world.xy(path[i]));
            let diag = a.0 != b.0 && a.1 != b.1;
            s[i] = s[i - 1] + step_len(if diag { 5 } else { 1 }) * spacing_m;
        }
        let (len, z1) = (s[path.len() - 1], z(*path.last()?));
        let line = |i: usize| zp + ((z1 - zp) as f64 * s[i] / len) as f32;
        let depth = (0..path.len())
            .map(|i| z(path[i]) - line(i))
            .fold(0.0f32, f32::max);
        if depth <= 0.0 || depth > c.cap {
            return None;
        }
        if len < f64::from(GORGE_RATIO * depth) {
            path.push(receiver(*path.last()?).filter(|&n| near(n))?);
            continue;
        }
        // The section: a Euclidean disc of radius half + 1 cells around each path cell with a
        // parabolic bed, the line at the centre rising to the ground at the edge (M1), so the
        // gorge is at least 3 cells wide and has no square, stair-stepped walls; its sides stay
        // for the relaxation.
        let mut new: HashMap<u64, f32> = HashMap::new();
        let radius = c.half as f32 + 1.0;
        let h = c.half as i64 + 1;
        for (i, &p) in path.iter().enumerate() {
            let (px, py) = world.xy(p);
            for dy in -h..=h {
                for dx in -h..=h {
                    let r = ((dx * dx + dy * dy) as f32).sqrt();
                    if r >= radius {
                        continue;
                    }
                    let (Some(x), Some(y)) = (px.checked_add_signed(dx), py.checked_add_signed(dy))
                    else {
                        continue;
                    };
                    if x >= world.w || y >= world.h {
                        continue;
                    }
                    let cell = world.index(x, y);
                    if !near(cell) {
                        return None;
                    }
                    let (ground, centre) = (z(cell), line(i));
                    let bed = centre + (ground - centre).max(0.0) * (r / radius).powi(2);
                    if bed < ground {
                        let e = new.entry(cell).or_insert(bed);
                        *e = e.min(bed);
                    }
                }
            }
        }
        let volume_mm: i64 = new.iter().map(|(&cell, &v)| mm(z(cell)) - mm(v)).sum();
        if volume_mm >= c.vfill_mm {
            return None;
        }
        let mut cut: Vec<(u64, f32)> = new.into_iter().collect();
        cut.sort_unstable_by_key(|p| p.0);
        return Some(Gorge {
            key: c.key,
            cut,
            depth,
            volume_mm,
            length_m: len,
        });
    }
}

/// Breach figures of one gorge: (node key, deepest cut, volume, length, spill cell).
type Figure = (u64, f32, i64, f64, u64);

/// Writes `ground` with every gorge cut to `out` (cores only) and returns the breach figures:
/// count, deepest cut, total volume in m³ and the 20 deepest with their cells.
pub(crate) fn breach(
    s: &Setup,
    m: &Model,
    ground: &Layer<f32>,
    out: &Layer<f32>,
    geo: &Geometry,
    work: Option<&Work>,
    tag: &str,
) -> Result<Value> {
    let world = s.world;
    let cands = candidates(s, m, geo);
    // Every gorge cell lies within the radius of its spill, so a tile takes the candidates
    // whose spill is that close and reads every cell within the radius + 1 of those spills.
    let reach = s.params.breach_radius;
    let halo = 2 * reach + 1;
    let per_tile: Vec<Vec<Figure>> = for_tiles(
        &s.tiles(),
        s.threads,
        &format!("{tag} breach"),
        |i, core| {
            let name = format!("{tag}/breach/{i:05}");
            if let Some(wk) = work {
                if wk.is_done(&name) {
                    return wk.cached(&format!("{name}-figures"), || Ok(Vec::new()));
                }
            }
            let win = core.grow(halo, world);
            let zw = ground.read(win)?;
            let mut new: Vec<f32> = crop(win, &zw, core);
            let reach_rect = core.grow(reach, world);
            let mut figures = Vec::new();
            for c in &cands {
                let (x, y) = world.xy(c.inside);
                if !reach_rect.contains(x, y) {
                    continue;
                }
                let Some(g) = gorge(world, win, &zw, c, s.spacing_m) else {
                    continue;
                };
                for &(cell, v) in &g.cut {
                    let (cx, cy) = world.xy(cell);
                    if let Some(k) = core.local(cx, cy) {
                        new[k] = new[k].min(v);
                    }
                }
                if core.contains(x, y) {
                    figures.push((g.key, g.depth, g.volume_mm, g.length_m, c.inside));
                }
            }
            out.write(core, &new)?;
            if let Some(wk) = work {
                wk.cached(&format!("{name}-figures"), || Ok(figures.clone()))?;
                wk.mark_done(&name)?;
            }
            Ok(figures)
        },
    )?;
    let mut all: Vec<Figure> = per_tile.into_iter().flatten().collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let cell_m2 = s.spacing_m * s.spacing_m;
    let m3 = |v: i64| v as f64 / 1000.0 * cell_m2;
    Ok(json!({
        "breaches": all.len(),
        "deepest_cut_m": all.first().map_or(0.0, |b| b.1),
        "volume_m3": all.iter().map(|b| m3(b.2)).sum::<f64>().round(),
        "deepest": all.iter().take(20).map(|b| {
            let (x, y) = world.xy(b.4);
            json!({
                "node": b.0,
                "cut_m": b.1,
                "volume_m3": m3(b.2).round(),
                "length_m": b.3,
                "spill_cell": [x, y],
            })
        }).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::pipeline::{Rasters, model};
    use crate::world::water::testing::setup;

    /// A V valley `w` cells long falling east 5 cm a cell with sides of 1:5 (a steeper V holds
    /// less than a 3-cell gorge costs), walled at its west end (so the pond can only leave
    /// over the dam, N8) and dammed at `at` by a one-cell wall `dam` m high at its centre,
    /// whose crest rises 2 cm a row (a flat crest ties with the valley sides, and the spill
    /// would wander along it).
    fn dammed_at(w: u64, at: f32, dam: f32) -> (World, Vec<f32>) {
        let world = World { w, h: 41 };
        let z = (0..world.w * world.h)
            .map(|i| {
                let (x, y) = ((i % world.w) as f32, (i / world.w) as f32);
                let side = (y - 20.0).abs();
                let v = 10.0 - 0.05 * x + 0.2 * side + if x <= 1.0 { 30.0 } else { 0.0 };
                if x == at && side <= 16.0 {
                    10.0 - 0.05 * (at - 1.0) + dam + 0.02 * side
                } else {
                    v
                }
            })
            .collect();
        (world, z)
    }

    fn dammed(dam: f32) -> (World, Vec<f32>) {
        dammed_at(100, 40.0, dam)
    }

    fn the_candidate(world: World, z: &[f32], cap: f32) -> (Candidate, Rect) {
        let mut s = setup(world, 128, 1e-5);
        s.classes = None;
        s.params.breach_cap_m = cap;
        let m = model(&s, &Layer::memory(world, z.to_vec()), None, "m", None).expect("model");
        let geo = Geometry::new(2.0);
        let c = candidates(&s, &m, &geo)
            .into_iter()
            .max_by_key(|c| c.vfill_mm)
            .expect("the pond behind the dam");
        (c, Rect::whole(world))
    }

    #[test]
    fn a_narrow_dam_is_cut_as_a_graded_gorge() {
        let (world, z) = dammed(1.0);
        let (c, all) = the_candidate(world, &z, 2.0);
        let g = gorge(world, all, &z, &c, 1.0).expect("breached");
        assert!(g.depth > 0.9 && g.depth <= 2.0, "depth {}", g.depth);
        assert!(
            g.length_m >= f64::from(GORGE_RATIO * g.depth),
            "length {} for depth {}",
            g.length_m,
            g.depth
        );
        // At least 3 neighbouring dam cells are cut (the gorge is 3 cells wide, not a slot).
        let cut_dam: Vec<u64> = g
            .cut
            .iter()
            .filter(|&&(c, v)| c % 100 == 40 && v < z[c as usize] - 0.5)
            .map(|p| p.0 / 100)
            .collect();
        assert!(cut_dam.len() >= 3, "{cut_dam:?}");
        assert!(
            cut_dam.windows(2).all(|p| p[1] == p[0] + 1),
            "adjacent rows: {cut_dam:?}"
        );
    }

    #[test]
    fn a_dam_above_the_cap_or_worth_less_than_its_cut_stays() {
        let (world, z) = dammed(2.5);
        let (c, all) = the_candidate(world, &z, 2.0);
        // The pond leaves over the dam, not off the map: the cap is what refuses it.
        let (ix, _) = world.xy(c.inside);
        let (ox, _) = world.xy(c.outside);
        assert!(
            (39..=41).contains(&ix) && (39..=41).contains(&ox),
            "outlet at the dam: {ix} {ox}"
        );
        assert!(
            gorge(world, all, &z, &c, 1.0).is_none(),
            "a 2.55 m cut, 2 m cap"
        );
        let mut wide = c;
        wide.cap = 3.0;
        assert!(
            gorge(world, all, &z, &wide, 1.0).is_some(),
            "the same dam is cut under a 3 m cap"
        );
        let (world, z) = dammed(1.0);
        let (mut c, all) = the_candidate(world, &z, 2.0);
        c.vfill_mm = 1;
        assert!(
            gorge(world, all, &z, &c, 1.0).is_none(),
            "the cut would remove more than it drains"
        );
    }

    #[test]
    fn breaching_is_alike_at_every_tiling() {
        // A dam on the tile seam at x = 64; a breach radius of 24 keeps the windows (tile + 49)
        // smaller than the world, so the gorge is cut across tiles (N9).
        let (world, z) = dammed_at(128, 64.0, 1.0);
        let ground = Layer::memory(world, z.clone());
        let mut out = Vec::new();
        for tile in [128, 32] {
            let mut s = setup(world, tile, 1e-5);
            s.classes = None;
            s.params.breach_radius = 24;
            let m = model(&s, &ground, None, "m", Some(&Rasters::memory(world))).expect("model");
            let breached = Layer::filled(world, 0.0f32);
            let figures =
                breach(&s, &m, &ground, &breached, &Geometry::new(2.0), None, "b").expect("breach");
            assert_eq!(figures["breaches"], 1, "tile {tile}");
            out.push(breached.to_vec().unwrap());
        }
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out[0]), bits(&out[1]));
        let cut: Vec<u64> = (0..z.len())
            .filter(|&k| out[1][k] < z[k])
            .map(|k| k as u64 % 128)
            .collect();
        assert!(
            cut.iter().any(|&x| x < 64) && cut.iter().any(|&x| x >= 64),
            "the gorge crosses the seam"
        );
    }
    /// Expert review: the model, breach and the model again on a 1,024² raw window at E43966
    /// N21633 (the staircase), tiles of 256², the layout's classes, the table's width factor,
    /// river start 0.05 km², uniform rain. Prints the breach figures and node counts before and
    /// after; writes t11-e43966-before.png and -after.png (lakes blue, rivers cyan, cut cells
    /// red by depth) and a cut-depth cross-section through the deepest spill.
    /// cargo test --release … breach -- --ignored --nocapture, under systemd-run MemoryMax=6G.
    #[test]
    #[ignore]
    fn probe_breach_at_e43966() {
        use crate::world::water::layers::Classes;
        use crate::world::water::probe;
        use crate::world::water::table::{DEFAULT, Table};
        let Some((win, z)) = probe::window(43966, 21633, 1024) else {
            return;
        };
        let Some(class) = probe::classes(win) else {
            return;
        };
        let world = World {
            w: win.w as u64,
            h: win.h as u64,
        };
        let mut s = setup(world, 256, 0.05);
        s.table = Table::parse(DEFAULT).expect("table");
        s.nw_m = [win.x0 as f64, (probe::WORLD.h - win.y0) as f64];
        s.classes = Some(Classes::new(Layer::memory(world, class), 1, world).expect("classes"));
        let geo = Geometry::new(s.table.m3s_per_km2);
        let run = |name: &str, ground: &[f32]| {
            let r = Rasters::memory(world);
            let m = model(
                &s,
                &Layer::memory(world, ground.to_vec()),
                None,
                "m",
                Some(&r),
            )
            .expect("model");
            let (node, flow) = (r.node.to_vec().unwrap(), r.flow.to_vec().unwrap());
            let lake = |k: usize| node[k] >= 2 && m.water.nodes.at(node[k] - 2).lake;
            let lakes = m.water.nodes.0.iter().filter(|n| n.lake).count();
            let full = m.water.nodes.0.iter().filter(|n| n.lake && n.full).count();
            let lake_cells = (0..ground.len()).filter(|&k| lake(k)).count();
            let rivers = flow.iter().filter(|&&f| f & 16 != 0).count();
            eprintln!(
                "{name}: nodes {} lakes {lakes} (full {full}) lake cells {lake_cells} river cells \
                 {rivers}",
                m.water.nodes.0.len()
            );
            let tint: Vec<Option<[u8; 3]>> = (0..ground.len())
                .map(|k| {
                    let cut = z[k] - ground[k];
                    if cut > 0.01 {
                        let red = (6.0 * cut / 2.0).clamp(1.0, 6.0) as u8;
                        Some([6, 6 - red, 6 - red])
                    } else if lake(k) {
                        Some([2, 3, 6])
                    } else if flow[k] & 16 != 0 {
                        Some([1, 5, 6])
                    } else {
                        None
                    }
                })
                .collect();
            probe::shade_tinted(name, ground, win.w, |k| tint[k]);
            m
        };
        let m = run("t11-e43966-before", &z);
        eprintln!(
            "candidates (full nodes a river flows into): {}",
            candidates(&s, &m, &geo).len()
        );
        let ground = Layer::memory(world, z.clone());
        let out = Layer::filled(world, 0.0f32);
        let figures = breach(&s, &m, &ground, &out, &geo, None, "b").expect("breach");
        eprintln!(
            "breach figures: {}",
            serde_json::to_string_pretty(&figures).unwrap()
        );
        let after = out.to_vec().unwrap();
        assert!(
            (0..z.len()).all(|k| after[k] <= z[k]),
            "ground is never raised"
        );
        let cut = (0..z.len()).filter(|&k| after[k] < z[k]).count();
        eprintln!("cut cells {cut}");
        run("t11-e43966-after", &after);
        // A cross-section (cut depth, m) across the deepest gorge at its spill, rows and columns.
        if let Some(d) = figures["deepest"].as_array().and_then(|a| a.first()) {
            let (sx, sy) = (
                d["spill_cell"][0].as_u64().unwrap(),
                d["spill_cell"][1].as_u64().unwrap(),
            );
            for (label, horizontal) in [("row", true), ("column", false)] {
                let line: Vec<String> = (-8i64..=8)
                    .filter_map(|o| {
                        let (x, y) = if horizontal {
                            (sx.checked_add_signed(o)?, sy)
                        } else {
                            (sx, sy.checked_add_signed(o)?)
                        };
                        let k = world.index(x.min(world.w - 1), y.min(world.h - 1)) as usize;
                        Some(format!("{:.2}", z[k] - after[k]))
                    })
                    .collect();
                eprintln!(
                    "deepest spill ({sx}, {sy}) E{} N{}: cut along the {label}: {}",
                    win.x0 + sx,
                    probe::WORLD.h - 1 - (win.y0 + sy),
                    line.join(" ")
                );
            }
        }
    }
}
