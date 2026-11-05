//! Flow inside a tile and across tiles (Barnes 2017): where in-tile paths end, upstream sums
//! along a forest of next cells, and the exact solve over the tiles' ports (ring cells and
//! node outlets). Amounts are u64, so no result depends on the tiling.

use super::store::{Blob, Dec, Enc};
use crate::acquisition::{AcquisitionError, Result, ensure};
use std::collections::{BTreeMap, HashMap};

/// Where flow goes from a cell: a cell of this tile, the first cell outside the tile (world
/// index), a node (a lake, a pit or the world edge, by key), or nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Next {
    Cell(u32),
    Out(u64),
    Node(u64),
    End,
}

impl Blob for Next {
    fn put(&self, e: &mut Enc) {
        let (tag, v) = match *self {
            Next::Cell(c) => (0u8, u64::from(c)),
            Next::Out(c) => (1, c),
            Next::Node(k) => (2, k),
            Next::End => (3, 0),
        };
        tag.put(e);
        v.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (tag, v) = (u8::get(d)?, u64::get(d)?);
        Ok(match tag {
            0 => Next::Cell(v as u32),
            1 => Next::Out(v),
            2 => Next::Node(v),
            3 => Next::End,
            _ => return Err(AcquisitionError::Invalid("work file: bad Next".into())),
        })
    }
}

/// For every cell, where its in-tile path ends (`Out`, `Node` or `End`, never `Cell`), and
/// how many cells lie on cycles (their end is `End`).
pub(crate) fn ends(n: usize, next: impl Fn(usize) -> Next) -> (Vec<Next>, u64) {
    const UNSEEN: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;
    let mut state = vec![UNSEEN; n];
    let mut end = vec![Next::End; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut cycles = 0u64;
    for start in 0..n {
        if state[start] != UNSEEN {
            continue;
        }
        let mut k = start;
        let found = loop {
            state[k] = OPEN;
            stack.push(k);
            match next(k) {
                Next::Cell(j) => {
                    let j = j as usize;
                    match state[j] {
                        UNSEEN => k = j,
                        DONE => break end[j],
                        _ => {
                            let from = stack.iter().rposition(|&c| c == j).unwrap_or(0);
                            cycles += (stack.len() - from) as u64;
                            break Next::End;
                        }
                    }
                }
                other => break other,
            }
        };
        for c in stack.drain(..) {
            end[c] = found;
            state[c] = DONE;
        }
    }
    (end, cycles)
}

/// Upstream sums of a tile.
pub(crate) struct Local {
    /// Each cell's rain plus everything upstream of it inside the tile.
    pub q: Vec<u64>,
    /// What leaves the tile, by the outside cell it enters.
    pub out: BTreeMap<u64, u64>,
    /// What enters nodes, by node key.
    pub node_in: BTreeMap<u64, u64>,
    /// Cells never reached in topological order (on or below a cycle).
    pub cycles: u64,
}

/// Sums `rain` along `next` in topological (Kahn) order, upstream first.
pub(crate) fn accumulate(
    n: usize,
    next: impl Fn(usize) -> Next,
    rain: impl Fn(usize) -> u64,
) -> Local {
    let mut indeg = vec![0u8; n];
    for k in 0..n {
        if let Next::Cell(j) = next(k) {
            indeg[j as usize] += 1;
        }
    }
    let mut q: Vec<u64> = (0..n).map(&rain).collect();
    let mut order: Vec<u32> = (0..n as u32).filter(|&k| indeg[k as usize] == 0).collect();
    let (mut out, mut node_in) = (BTreeMap::new(), BTreeMap::new());
    let mut head = 0;
    while head < order.len() {
        let k = order[head] as usize;
        head += 1;
        match next(k) {
            Next::Cell(j) => {
                let j = j as usize;
                q[j] += q[k];
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    order.push(j as u32);
                }
            }
            Next::Out(c) => *out.entry(c).or_insert(0) += q[k],
            Next::Node(key) => *node_in.entry(key).or_insert(0) += q[k],
            Next::End => {}
        }
    }
    Local {
        q,
        out,
        node_in,
        cycles: (n - order.len()) as u64,
    }
}

/// A tile's flow report for the solve across tiles.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct Report {
    pub out: BTreeMap<u64, u64>,
    pub node_in: BTreeMap<u64, u64>,
    /// Where the in-tile path from each port (ring cells, node outlets in the tile) ends.
    pub port_end: BTreeMap<u64, Next>,
}

impl Blob for Report {
    fn put(&self, e: &mut Enc) {
        (
            self.out.clone(),
            self.node_in.clone(),
            self.port_end.clone(),
        )
            .put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (out, node_in, port_end) = Blob::get(d)?;
        Ok(Self {
            out,
            node_in,
            port_end,
        })
    }
}

#[derive(Default)]
pub(crate) struct Solved {
    /// What enters each port from beyond its own tile's cells.
    pub inflow: HashMap<u64, u64>,
    /// Each node's whole inflow.
    pub node_total: BTreeMap<u64, u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum V {
    Port(u64),
    Node(u64),
}

/// The exact totals across tiles. `outlet` gives the cell a node's water leaves by (None: the
/// node keeps it). Errors on a cycle or on flow into a cell no tile reported.
pub(crate) fn solve(reports: &[Report], outlet: &BTreeMap<u64, Option<u64>>) -> Result<Solved> {
    let mut port_end: HashMap<u64, Next> = HashMap::new();
    let mut inflow: HashMap<u64, u64> = HashMap::new();
    let mut node_total: BTreeMap<u64, u64> = outlet.keys().map(|&k| (k, 0)).collect();
    for r in reports {
        port_end.extend(r.port_end.iter().map(|(&c, &e)| (c, e)));
        for (&c, &q) in &r.out {
            *inflow.entry(c).or_insert(0) += q;
        }
        for (&k, &q) in &r.node_in {
            *node_total.entry(k).or_insert(0) += q;
        }
    }
    // Water leaving a tile must enter a reported port, or it would vanish.
    ensure(
        inflow.keys().all(|c| port_end.contains_key(c)),
        "flow leaves into a cell no tile reported",
    )?;
    for e in port_end.values() {
        if let Next::Node(k) = e {
            node_total.entry(*k).or_insert(0);
        }
    }
    let target = |v: V| -> Option<V> {
        match v {
            V::Port(c) => match port_end.get(&c) {
                Some(Next::Out(o)) => Some(V::Port(*o)),
                Some(Next::Node(k)) => Some(V::Node(*k)),
                _ => None,
            },
            V::Node(k) => outlet.get(&k).copied().flatten().map(V::Port),
        }
    };
    let verts: Vec<V> = port_end
        .keys()
        .map(|&c| V::Port(c))
        .chain(node_total.keys().map(|&k| V::Node(k)))
        .collect();
    let mut indeg: HashMap<V, u32> = verts.iter().map(|&v| (v, 0)).collect();
    for &v in &verts {
        if let Some(t) = target(v) {
            *indeg.get_mut(&t).ok_or_else(|| {
                AcquisitionError::Invalid("flow leaves into a cell no tile reported".into())
            })? += 1;
        }
    }
    let mut queue: Vec<V> = verts.iter().copied().filter(|v| indeg[v] == 0).collect();
    let mut done = 0usize;
    while let Some(v) = queue.pop() {
        done += 1;
        let amount = match v {
            V::Port(c) => inflow.get(&c).copied().unwrap_or(0),
            V::Node(k) => node_total[&k],
        };
        if let Some(t) = target(v) {
            match t {
                V::Port(c) => *inflow.entry(c).or_insert(0) += amount,
                V::Node(k) => *node_total.get_mut(&k).expect("node vertex") += amount,
            }
            let d = indeg.get_mut(&t).expect("vertex");
            *d -= 1;
            if *d == 0 {
                queue.push(t);
            }
        }
    }
    ensure(done == verts.len(), "flow between tiles has a cycle")?;
    Ok(Solved { inflow, node_total })
}

/// Adds what enters each port (local index, amount) to every cell on its in-tile path.
pub(crate) fn add_inflows(q: &mut [u64], ports: &[(usize, u64)], next: impl Fn(usize) -> Next) {
    for &(k, a) in ports {
        if a == 0 {
            continue;
        }
        let mut c = k;
        for _ in 0..q.len() {
            q[c] += a;
            match next(c) {
                Next::Cell(j) => c = j as usize,
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::water::geom::{Rect, World, tiles};
    use crate::world::water::store::decode;

    /// A slope falling east with noise; cells of the block x 5..9, y 5..9 are node 7, which
    /// leaves by (10, 7). Receivers: the lowest lower neighbour; edge cells end.
    fn case(world: World) -> (Vec<f32>, impl Fn(u64, u64) -> Option<Next>) {
        let z: Vec<f32> = (0..world.w * world.h)
            .map(|i| 100.0 - (i % world.w) as f32 + ((i * 7919) % 13) as f32 * 0.01)
            .collect();
        let zz = z.clone();
        let next = move |x: u64, y: u64| -> Option<Next> {
            if (5..10).contains(&x) && (5..10).contains(&y) {
                return Some(Next::Node(7));
            }
            if world.is_edge(x, y) {
                return Some(Next::End);
            }
            let here = zz[world.index(x, y) as usize];
            let mut best: Option<(f32, u64)> = None;
            for k in 0..8 {
                let (nx, ny) = world.neighbour(x, y, k)?;
                let v = zz[world.index(nx, ny) as usize];
                if v < here && best.is_none_or(|b| v < b.0) {
                    best = Some((v, world.index(nx, ny)));
                }
            }
            Some(best.map_or(Next::End, |b| Next::Out(b.1)))
        };
        (z, next)
    }

    /// Discharge on every cell from tiles of `size` (whole world when size >= its side).
    fn discharge(world: World, size: u64) -> Vec<u64> {
        let (_, global) = case(world);
        let outlet = BTreeMap::from([(7u64, Some(world.index(10, 7)))]);
        let rain = |_: usize| 3u64;
        let tiles = tiles(world, size);
        let local_next = |core: Rect| {
            let g = &global;
            move |k: usize| {
                let (x, y) = core.world_xy(k);
                match g(x, y).expect("cell") {
                    Next::Out(c) => {
                        let (cx, cy) = world.xy(c);
                        core.local(cx, cy)
                            .map_or(Next::Out(c), |j| Next::Cell(j as u32))
                    }
                    other => other,
                }
            }
        };
        let mut reports = Vec::new();
        for &core in &tiles {
            let next = local_next(core);
            let local = accumulate(core.cells(), next, rain);
            let (end, cycles) = ends(core.cells(), next);
            assert_eq!(cycles + local.cycles, 0);
            let mut port_end = BTreeMap::new();
            for (k, &e) in end.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                if core.on_ring(x, y) || world.index(x, y) == world.index(10, 7) {
                    port_end.insert(world.index(x, y), e);
                }
            }
            reports.push(Report {
                out: local.out,
                node_in: local.node_in,
                port_end,
            });
        }
        let solved = solve(&reports, &outlet).expect("solve");
        let mut q = vec![0u64; (world.w * world.h) as usize];
        for &core in &tiles {
            let next = local_next(core);
            let mut local = accumulate(core.cells(), next, rain).q;
            let ports: Vec<(usize, u64)> = (0..core.cells())
                .filter_map(|k| {
                    let (x, y) = core.world_xy(k);
                    solved.inflow.get(&world.index(x, y)).map(|&a| (k, a))
                })
                .collect();
            add_inflows(&mut local, &ports, next);
            for (k, &lq) in local.iter().enumerate() {
                let (x, y) = core.world_xy(k);
                q[world.index(x, y) as usize] = match next(k) {
                    Next::Node(n) => solved.node_total[&n],
                    _ => lq,
                };
            }
        }
        q
    }

    #[test]
    fn tiled_sums_equal_the_whole() {
        let world = World { w: 32, h: 24 };
        let whole = discharge(world, 64);
        assert_eq!(discharge(world, 8), whole);
        assert_eq!(discharge(world, 5), whole);
        // Mass: every cell's water ends in a cell whose next is End (the edge or a dry pit), so
        // their discharges sum to all the rain (3 per cell) (N9).
        let (_, global) = case(world);
        let arriving: u64 = (0..world.w * world.h)
            .filter(|&i| global(i % world.w, i / world.w) == Some(Next::End))
            .map(|i| whole[i as usize])
            .sum();
        assert_eq!(arriving, 3 * world.w * world.h);
        assert!(
            whole.iter().any(|&q| q > 3 * 50),
            "the node's water flows on"
        );
    }

    #[test]
    fn ends_report_cycles() {
        let next = |k: usize| match k {
            0 => Next::Cell(1),
            1 => Next::Cell(0),
            2 => Next::Cell(0),
            _ => Next::Out(99),
        };
        let (end, cycles) = ends(4, next);
        assert_eq!(cycles, 2);
        assert_eq!(end[3], Next::Out(99));
        assert_eq!(end[2], Next::End);
    }

    #[test]
    fn a_cycle_between_tiles_is_an_error() {
        let a = Report {
            port_end: BTreeMap::from([(1, Next::Out(2))]),
            ..Default::default()
        };
        let b = Report {
            port_end: BTreeMap::from([(2, Next::Out(1))]),
            ..Default::default()
        };
        assert!(solve(&[a, b], &BTreeMap::new()).is_err());
    }

    #[test]
    fn flow_into_an_unreported_cell_is_an_error() {
        let a = Report {
            out: BTreeMap::from([(5, 9)]),
            port_end: BTreeMap::from([(1, Next::End)]),
            ..Default::default()
        };
        assert!(solve(std::slice::from_ref(&a), &BTreeMap::new()).is_err());
        let to_node = BTreeMap::from([(7, Some(5))]);
        assert!(
            solve(&[Report::default()], &to_node).is_err(),
            "node outlet unreported"
        );
    }

    #[test]
    fn reports_round_trip() {
        let r = Report {
            out: BTreeMap::from([(5, 9)]),
            node_in: BTreeMap::from([(7, 3)]),
            port_end: BTreeMap::from([
                (1, Next::End),
                (2, Next::Out(u64::MAX)),
                (3, Next::Node(7)),
                (4, Next::Cell(u32::MAX)),
            ]),
        };
        let mut e = Enc(Vec::new());
        r.put(&mut e);
        let back: Report = decode(&e.0).expect("decode");
        assert_eq!(back, r);
    }
}
