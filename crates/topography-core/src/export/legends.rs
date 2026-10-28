//! Class legends of the biome and land-cover layers, keyed by stable names for the game.

use crate::acquisition::{AcquisitionError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const TERRAIN: &str = include_str!("../../../../catalog/labels/terrain-v3.toml");

/// ESA WorldCover v200 codes.
const WORLD_COVER: [(u8, &str, &str); 11] = [
    (10, "tree_cover", "Tree cover"),
    (20, "shrubland", "Shrubland"),
    (30, "grassland", "Grassland"),
    (40, "cropland", "Cropland"),
    (50, "built_up", "Built-up"),
    (60, "bare_sparse", "Bare / sparse vegetation"),
    (70, "snow_ice", "Snow and ice"),
    (80, "water", "Permanent water bodies"),
    (90, "herbaceous_wetland", "Herbaceous wetland"),
    (95, "mangroves", "Mangroves"),
    (100, "moss_lichen", "Moss and lichen"),
];

#[derive(Deserialize)]
struct Terrain {
    classes: Vec<TerrainClass>,
}

#[derive(Deserialize)]
struct TerrainClass {
    id: u8,
    key: Option<String>,
    name: String,
}

/// Legend entries with id 0 first; `present` marks classes with finest-level texels.
fn classes<'a>(
    counts: &[u64; 256],
    entries: impl Iterator<Item = (u8, &'a str, &'a str)>,
) -> Vec<Value> {
    std::iter::once((0, "unknown", "Unlabelled"))
        .chain(entries)
        .map(|(id, key, name)| {
            json!({"id": id, "key": key, "name": name, "present": counts[usize::from(id)] > 0})
        })
        .collect()
}

pub(super) fn legends(biome: &[u64; 256], land_cover: &[u64; 256]) -> Result<Value> {
    let terrain: Terrain = toml::from_str(TERRAIN)
        .map_err(|e| AcquisitionError::Invalid(format!("terrain-v3 legend: {e}")))?;
    let mut entries = Vec::new();
    for c in &terrain.classes {
        match c.key.as_deref() {
            Some(key) if !key.is_empty() => entries.push((c.id, key, c.name.as_str())),
            _ => {
                return Err(AcquisitionError::Invalid(format!(
                    "terrain-v3 class {} has no key",
                    c.id
                )));
            }
        }
    }
    Ok(json!({
        "biome": {"legend": "topography terrain-v3", "classes": classes(biome, entries.into_iter())},
        "land-cover": {"legend": "ESA WorldCover v200", "classes": classes(land_cover, WORLD_COVER.into_iter())},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legends_name_every_class_and_mark_presence() {
        let mut b = [0u64; 256];
        b[6] = 10;
        b[19] = 1;
        let mut c = [0u64; 256];
        c[10] = 5;
        let v = legends(&b, &c).unwrap();
        let biome = v["biome"]["classes"].as_array().unwrap();
        assert_eq!(biome[0]["key"], "unknown");
        let alpine = biome.iter().find(|c| c["id"] == 6).unwrap();
        assert_eq!(
            (alpine["key"].as_str(), alpine["present"].as_bool()),
            (Some("alpine_ridges"), Some(true))
        );
        assert!(
            biome
                .iter()
                .all(|c| c["key"].as_str().is_some_and(|k| !k.is_empty()))
        );
        let tree = v["land-cover"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == 10)
            .unwrap();
        assert_eq!(tree["key"], "tree_cover");
    }
}
