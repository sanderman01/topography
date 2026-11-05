//! Every water node of a run (a depression whose water covers the cells under it: a lake, a
//! pond or a closed basin), from the global hierarchy and from the trees settled in tiles,
//! named by its smallest pit cell (R28): the same name at any tiling and with contraction.

use super::store::{Blob, Dec, Enc};
use crate::acquisition::Result;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Node {
    pub key: u64,
    pub level: f32,
    pub low: f32,
    pub full: bool,
    pub lake: bool,
    /// [cell inside, cell outside] of its spill edge (meaningful when full).
    pub outlet: [u64; 2],
    /// Cells its water covers.
    pub cells: u64,
    pub vfill_mm: i64,
    /// Closed nodes: cells the balance needs.
    pub need: u64,
    pub bbox: [u64; 4],
}

/// The node table, sorted by key.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Nodes(pub Vec<Node>);

impl Nodes {
    pub fn ordinal(&self, key: u64) -> Option<u32> {
        self.0
            .binary_search_by_key(&key, |n| n.key)
            .ok()
            .map(|i| i as u32)
    }
    pub fn get(&self, key: u64) -> Option<&Node> {
        self.ordinal(key).map(|o| &self.0[o as usize])
    }
    pub fn at(&self, ordinal: u32) -> &Node {
        &self.0[ordinal as usize]
    }
}

impl Blob for Node {
    fn put(&self, e: &mut Enc) {
        (self.key, self.level, self.low, self.full, self.lake).put(e);
        (self.outlet, self.cells, self.vfill_mm, self.need, self.bbox).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let (key, level, low, full, lake) = Blob::get(d)?;
        let (outlet, cells, vfill_mm, need, bbox) = Blob::get(d)?;
        Ok(Self {
            key,
            level,
            low,
            full,
            lake,
            outlet,
            cells,
            vfill_mm,
            need,
            bbox,
        })
    }
}

impl Blob for Nodes {
    fn put(&self, e: &mut Enc) {
        self.0.put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let rows: Vec<Node> = Blob::get(d)?;
        crate::acquisition::ensure(
            rows.windows(2).all(|w| w[0].key < w[1].key),
            "node table keys are not sorted and unique",
        )?;
        Ok(Self(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_found_by_key_and_ordinal() {
        let n = |key| Node {
            key,
            ..Default::default()
        };
        let t = Nodes(vec![n(3), n(10), n(42)]);
        assert_eq!(t.ordinal(10), Some(1));
        assert_eq!(t.ordinal(11), None);
        assert_eq!(t.at(2).key, 42);
        assert_eq!(t.get(3).map(|x| x.key), Some(3));
        assert!(t.get(4).is_none());
    }

    #[test]
    fn node_tables_round_trip_and_unsorted_ones_are_refused() {
        use crate::world::water::store::{decode, encode};
        let t = Nodes(vec![
            Node {
                key: 7,
                level: 3.5,
                low: 1.0,
                full: true,
                lake: true,
                outlet: [8, 9],
                cells: 12,
                vfill_mm: -4,
                need: 0,
                bbox: [1, 2, 3, 4],
            },
            Node {
                key: 9,
                need: 5,
                ..Default::default()
            },
        ]);
        assert_eq!(decode::<Nodes>(&encode(&t)).expect("decode"), t);
        let bad = Nodes(vec![t.0[1], t.0[0]]);
        assert!(decode::<Nodes>(&encode(&bad)).is_err());
    }
}
