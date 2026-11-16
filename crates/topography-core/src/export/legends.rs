//! Class legends of the biome and land-cover layers, keyed by stable names for the game.

use crate::acquisition::{AcquisitionError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const TERRAIN: &str = include_str!("../../../../catalog/labels/terrain-v3.toml");

/// ESA WorldCover v200 codes and colours, plus topography extension classes.
const WORLD_COVER: [(u8, &str, &str, &str); 12] = [
    (10, "tree_cover", "Tree cover", "#006400"),
    (20, "shrubland", "Shrubland", "#ffbb22"),
    (21, "riparian", "Riparian vegetation", "#6fbf3f"),
    (30, "grassland", "Grassland", "#ffff4c"),
    (40, "cropland", "Cropland", "#f096ff"),
    (50, "built_up", "Built-up", "#fa0000"),
    (60, "bare_sparse", "Bare / sparse vegetation", "#b4b4b4"),
    (70, "snow_ice", "Snow and ice", "#f0f0f0"),
    (80, "water", "Permanent water bodies", "#0064c8"),
    (90, "herbaceous_wetland", "Herbaceous wetland", "#0096a0"),
    (95, "mangroves", "Mangroves", "#00cf75"),
    (100, "moss_lichen", "Moss and lichen", "#fae6a0"),
];

/// Classes that are not in ESA WorldCover v200.
const EXTENSIONS: [u8; 1] = [21];

/// Land-cover classes drawn from a fraction layer: the layer is the better source for them.
const DERIVED: [(u8, &str); 3] = [
    (10, "vegetation-density (its forest part)"),
    (21, "vegetation-density (riparian band, density 0.25)"),
    (70, "snow-cover"),
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
    let mut cover = classes(
        land_cover,
        WORLD_COVER
            .iter()
            .map(|&(id, key, name, _)| (id, key, name)),
    );
    for c in &mut cover {
        let id = c["id"].as_u64().unwrap_or(0) as u8;
        if let Some((_, _, _, colour)) = WORLD_COVER.iter().find(|e| e.0 == id) {
            c["color"] = json!(colour);
        }
        if EXTENSIONS.contains(&id) {
            c["extension"] = json!(true);
        }
        if let Some((_, layer)) = DERIVED.iter().find(|e| e.0 == id) {
            c["derived_from"] = json!(layer);
        }
    }
    Ok(json!({
        "biome": {"legend": "topography terrain-v3", "classes": classes(biome, entries.into_iter())},
        "land-cover": {"legend": "ESA WorldCover v200 with topography extensions (extension: true)",
            "classes": cover},
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
        assert_eq!(tree["derived_from"], "vegetation-density (its forest part)");
        let cover = v["land-cover"]["classes"].as_array().unwrap();
        let riparian = cover.iter().find(|c| c["id"] == 21).unwrap();
        assert_eq!(
            (riparian["key"].as_str(), riparian["extension"].as_bool()),
            (Some("riparian"), Some(true))
        );
        assert!(cover.iter().skip(1).all(|c| c["color"].is_string()));
    }
}
