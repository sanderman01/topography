//! Export checks gathered while writing, and the validation report.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Examples kept per check.
const EXAMPLES: usize = 3;

#[derive(Default)]
pub(super) struct Checks {
    /// Level 0: body > 0 with a finite surface at or below the ground.
    pub wet_not_above_ground: u64,
    pub wet_without_surface: u64,
    pub surface_without_body: u64,
    /// Level 0: a lake or sea cell whose raster surface does not encode to its level's code.
    pub lake_levels_off: u64,
    /// (level, export id) → min and max surface code written for that lake or sea.
    pub lake_levels: BTreeMap<(u32, u16), (u16, u16)>,
    /// Level 0: coverage texel ≠ 255·(body > 0 and surface code ≥ ground code).
    pub coverage_mismatch: u64,
    pub shared_edge_mismatches: u64,
    /// Index level − 1: texels with coverage ≥ 0.5 and surface below the mean ground.
    pub coarse_wet_below_ground: Vec<u64>,
    /// Baked normal texels off by more than 30° after BC5, or roughness by more than 0.25.
    pub bc_gross_errors: u64,
    /// Measured BC5/BC4 error per level, when normals are exported.
    pub bc_budget: Option<Value>,
    examples: BTreeMap<&'static str, Vec<String>>,
}

impl Checks {
    /// Checks of an export whose last level is `last`.
    pub fn new(last: u32) -> Self {
        Self {
            coarse_wet_below_ground: vec![0; last as usize],
            ..Self::default()
        }
    }

    /// Keeps the first three examples of each check.
    pub fn example(&mut self, check: &'static str, text: impl FnOnce() -> String) {
        let list = self.examples.entry(check).or_default();
        if list.len() < EXAMPLES {
            list.push(text());
        }
    }

    pub fn record_lake(&mut self, level: u32, id: u16, code: u16) {
        let e = self.lake_levels.entry((level, id)).or_insert((code, code));
        e.0 = e.0.min(code);
        e.1 = e.1.max(code);
    }

    /// Lakes or seas with more than one surface code at one level.
    fn split_lakes(&self) -> impl Iterator<Item = (&(u32, u16), &(u16, u16))> {
        self.lake_levels.iter().filter(|(_, (lo, hi))| lo != hi)
    }

    fn hard(&self) -> [(&'static str, u64); 8] {
        [
            ("wet_not_above_ground", self.wet_not_above_ground),
            ("wet_without_surface", self.wet_without_surface),
            ("surface_without_body", self.surface_without_body),
            ("lake_levels_off", self.lake_levels_off),
            // Guard: the reducer writes level_m for every lake texel, so this stays 0 today.
            ("lake_levels", self.split_lakes().count() as u64),
            ("coverage_mismatch", self.coverage_mismatch),
            ("shared_edge_mismatches", self.shared_edge_mismatches),
            ("bc_gross_errors", self.bc_gross_errors),
        ]
    }

    fn examples_of(&self, check: &str) -> Vec<String> {
        if check == "lake_levels" {
            return self
                .split_lakes()
                .take(EXAMPLES)
                .map(|((l, id), (lo, hi))| format!("L{l} export id {id}: codes {lo} to {hi}"))
                .collect();
        }
        self.examples.get(check).cloned().unwrap_or_default()
    }

    pub fn passed(&self) -> bool {
        self.hard().iter().all(|(_, n)| *n == 0)
    }

    /// Failed checks with their counts and examples, e.g.
    /// `wet_not_above_ground 1 (at (5, 5): ...)`.
    pub fn failures(&self) -> String {
        self.hard()
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(name, n)| format!("{name} {n} ({})", self.examples_of(name).join("; ")))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn report(&self) -> Value {
        let mut checks = Map::new();
        let mut examples = Map::new();
        for (name, n) in self.hard() {
            checks.insert(name.into(), json!(n));
            let list = self.examples_of(name);
            if !list.is_empty() {
                examples.insert(name.into(), json!(list));
            }
        }
        let coarse: Map<String, Value> = self
            .coarse_wet_below_ground
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("L{}", i + 1), json!(n)))
            .collect();
        let mut figures = json!({
            "coarse_wet_below_ground": coarse,
            "lake_levels_checked": self.lake_levels.len(),
        });
        if let Some(budget) = &self.bc_budget {
            figures["bc_error_budget"] = budget.clone();
        }
        json!({
            "checks": checks,
            "figures": figures,
            "examples": examples,
            "passed": self.passed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tiles::TileWriter;
    use super::super::{Layout, NormalOptions, heightmap, testworld};
    use super::*;
    use std::fs;

    #[test]
    fn a_wet_cell_at_or_below_ground_fails_the_export() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 64, 1.0, |x, y| {
            if (x, y) == (5, 5) {
                (10.0, 9.0, 10)
            } else {
                (20.0, f32::NAN, 0)
            }
        });
        let err = heightmap(
            &world,
            &dir.path().join("x"),
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap_err()
        .to_string();
        // Caught by the range scan, before any tile is written.
        assert!(
            err.contains("world checks failed: wet_not_above_ground 1 (at (5, 5)"),
            "{err}"
        );
        assert!(!dir.path().join("x").exists());
    }

    #[test]
    fn a_lake_surface_off_its_level_fails() {
        // Lake 2 over a 10 m bed has level 11 m; its raster surface says 13 and 13.5 m.
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 64, 1.0, |x, y| match (x, y) {
            (10..=19, 10..=19) => (10.0, if x < 15 { 13.0 } else { 13.5 }, 2),
            _ => (20.0, f32::NAN, 0),
        });
        let err = heightmap(
            &world,
            &dir.path().join("x"),
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains(
                "lake_levels_off 100 (at (10, 10): world body 2 surface 13 m, level 11 m;"
            ),
            "{err}"
        );
        assert!(!dir.path().join("x").exists());
    }

    #[test]
    fn a_lake_with_two_codes_at_one_level_fails() {
        let mut checks = Checks::default();
        checks.record_lake(0, 2, 7);
        checks.record_lake(1, 2, 7);
        assert!(checks.passed());
        checks.record_lake(1, 2, 8);
        assert!(!checks.passed());
        assert_eq!(
            checks.failures(),
            "lake_levels 1 (L1 export id 2: codes 7 to 8)"
        );
    }

    #[test]
    fn shared_edge_mismatch_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = TileWriter::new(dir.path(), true);
        let mut checks = Checks::default();
        // Wet notes of other checks must not take the shared-edge example slots.
        for i in 0..3 {
            checks.example("wet_not_above_ground", || format!("at ({i}, 0)"));
        }
        let spec = super::super::layers::BIOME;
        let west: Vec<u8> = (0..121).map(|i| (i % 11) as u8).collect();
        let mut east: Vec<u8> = (0..121).map(|i| (i % 11 + 2) as u8).collect();
        east[11] = 0;
        writer
            .write(&spec, 1, 0, 0, 11, 11, &west, &mut checks)
            .unwrap();
        writer
            .write(&spec, 1, 1, 0, 11, 11, &east, &mut checks)
            .unwrap();
        assert_eq!(checks.shared_edge_mismatches, 1);
        let report = checks.report();
        assert_eq!(report["checks"]["shared_edge_mismatches"], 1);
        assert_eq!(report["passed"], false);
        assert_eq!(
            report["examples"]["shared_edge_mismatches"],
            json!(["biome/L1/biome_L1_1_0.png: 1 texels differ from the west tile"])
        );
    }

    #[test]
    fn the_report_passes_on_a_clean_world() {
        // A lake, a river and a sea strip, over sloping ground.
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 128, 1.0, |x, y| {
            let g = 2.0 + 0.1 * x as f32;
            if x < 8 {
                (-3.0, f32::NAN, 1)
            } else if (40..60).contains(&x) && (40..60).contains(&y) {
                (g - 1.0 - (y % 3) as f32, f32::NAN, 2)
            } else if x == 80 {
                (g, g + 0.5, 10)
            } else {
                (g, f32::NAN, 0)
            }
        });
        let out = dir.path().join("x");
        heightmap(
            &world,
            &out,
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap();
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let v = &m["validation"];
        assert_eq!(v["passed"], true);
        for (name, n) in v["checks"].as_object().unwrap() {
            assert_eq!(n, 0, "{name}");
        }
        assert_eq!(v["examples"], json!({}));
        // Sea and lake at levels 0 to 3.
        assert_eq!(v["figures"]["lake_levels_checked"], 8);
        assert!(v["figures"]["coarse_wet_below_ground"]["L3"].is_u64());
        let report: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(&report, v);
    }
}
