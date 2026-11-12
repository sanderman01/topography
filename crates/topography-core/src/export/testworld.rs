//! Small synthetic world outputs for exporter tests.

use crate::acquisition::digest;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn put(world: &Path, manifest: &mut Value, name: &str, bytes: &[u8]) {
    let path = world.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    manifest["layers"][name] = json!(digest(bytes));
}

fn save(world: &Path, manifest: &Value) {
    fs::write(
        world.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
}

/// Writes an `n`² world at `spacing` m into `dir/world` and returns that directory.
/// `cell(x, y)` gives (ground, surface, body); body 1 is sea at level 0, 2..=9 lakes at the
/// ground maximum of their cells + 1 (both set their surface; a finite lake surface is kept),
/// 9 of them frozen (kind `ice`), others rivers. Body 0 is dry.
pub(crate) fn write(
    dir: &Path,
    n: usize,
    spacing: f64,
    cell: impl Fn(usize, usize) -> (f32, f32, u32),
) -> PathBuf {
    let world = dir.join("world");
    let input = world.join("world-input");
    fs::create_dir_all(&input).unwrap();
    let cells: Vec<_> = (0..n * n).map(|i| cell(i % n, i / n)).collect();
    let mut lake_level = BTreeMap::new();
    for &(g, _, b) in &cells {
        if (2..=9).contains(&b) {
            let level = lake_level.entry(b).or_insert(f32::MIN);
            *level = level.max(g + 1.0);
        }
    }
    let surface = |&(_, s, b): &(f32, f32, u32)| match b {
        0 => f32::NAN,
        1 => 0.0,
        2..=9 if s.is_nan() => lake_level[&b],
        _ => s,
    };
    let f32s =
        |v: &mut dyn Iterator<Item = f32>| -> Vec<u8> { v.flat_map(f32::to_le_bytes).collect() };
    let elevation = f32s(&mut cells.iter().map(|c| c.0));
    let surfaces = f32s(&mut cells.iter().map(surface));
    let body: Vec<u8> = cells.iter().flat_map(|c| c.2.to_le_bytes()).collect();
    let mut ids: Vec<u32> = cells.iter().map(|c| c.2).filter(|&b| b > 1).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut bodies = vec![json!({"id": 1, "kind": "sea", "level_m": 0.0})];
    bodies.extend(ids.iter().map(|&id| match lake_level.get(&id) {
        Some(level) => {
            let kind = if id == 9 { "ice" } else { "lake" };
            json!({"id": id, "kind": kind, "level_m": level, "parent": 0})
        }
        None => json!({"id": id, "kind": "river", "parent": 0}),
    }));
    let bodies = json!({"schema_version": 1, "kind": "water_bodies", "bodies": bodies});

    fs::write(
        input.join("world.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "testworld",
            "crs": "EPSG:32654",
            "origin": [0.0, n as f64 * spacing],
            "base": {"attribution": "synthetic"},
            "cover": {"attribution": "synthetic"},
        }))
        .unwrap(),
    )
    .unwrap();
    let mut manifest = json!({
        "schema_version": 1,
        "kind": "world_preview",
        "world_input": input.canonicalize().unwrap().display().to_string(),
        "spacing_m": spacing,
        "size": [n, n],
        "layers": {},
    });
    put(&world, &mut manifest, "elevation.f32", &elevation);
    put(&world, &mut manifest, "water-surface.f32", &surfaces);
    put(&world, &mut manifest, "water-body.u32", &body);
    put(
        &world,
        &mut manifest,
        "bodies.json",
        &serde_json::to_vec_pretty(&bodies).unwrap(),
    );
    save(&world, &manifest);
    world
}

/// Game layer names in the order `game` takes their values.
const GAME: [&str; 5] = [
    "biome",
    "land-cover",
    "vegetation-density",
    "snow-cover",
    "talus-density",
];

/// Adds the `game/` layers at `level` (size `n >> level`) and their manifest entry.
pub(crate) fn game(world: &Path, level: u32, f: impl Fn(usize, usize) -> [u8; 5]) {
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(world.join("manifest.json")).unwrap()).unwrap();
    let n = manifest["size"][0].as_u64().unwrap() as usize;
    let spacing = manifest["spacing_m"].as_f64().unwrap();
    let m = n >> level;
    let values: Vec<[u8; 5]> = (0..m * m).map(|i| f(i % m, i / m)).collect();
    for (k, name) in GAME.iter().enumerate() {
        let bytes: Vec<u8> = values.iter().map(|v| v[k]).collect();
        put(world, &mut manifest, &format!("game/{name}.u8"), &bytes);
    }
    manifest["game"] = json!({
        "spacing_m": spacing * f64::from(1u32 << level),
        "size": [m, m],
        "level": level,
        "lattice": format!("value (i, j) is the area mean of the footprint centred on world sample (i·2^{level}, j·2^{level})"),
        "method": "testworld",
        "params": {},
        "layers": GAME,
    });
    save(world, &manifest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_layers_bodies_and_game() {
        let dir = tempfile::tempdir().unwrap();
        let world = write(dir.path(), 4, 2.0, |x, y| match (x, y) {
            (0, _) => (-3.0, f32::NAN, 1),
            (1, 0) => (5.0, f32::NAN, 2),
            (1, 1) => (7.0, f32::NAN, 2),
            (2, 0) => (8.0, 9.5, 10),
            _ => (10.0, f32::NAN, 0),
        });
        game(&world, 1, |x, y| [(x + 2 * y) as u8, 30, 0, 0, 0]);
        let m: Value =
            serde_json::from_slice(&fs::read(world.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["layers"].as_object().unwrap().len(), 9);
        for (name, sha) in m["layers"].as_object().unwrap() {
            assert_eq!(
                sha.as_str().unwrap(),
                digest(&fs::read(world.join(name)).unwrap())
            );
        }
        assert_eq!(m["game"]["size"], json!([2, 2]));
        assert_eq!(fs::read(world.join("game/biome.u8")).unwrap(), [0, 1, 2, 3]);
        let surface: Vec<f32> = fs::read(world.join("water-surface.f32"))
            .unwrap()
            .chunks(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(&surface[..3], [0.0, 8.0, 9.5]);
        assert_eq!(surface[5], 8.0);
        assert!(surface[3].is_nan());
        let bodies: Value =
            serde_json::from_slice(&fs::read(world.join("bodies.json")).unwrap()).unwrap();
        let kinds: Vec<_> = bodies["bodies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["id"].as_u64().unwrap(),
                    b["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [(1, "sea".into()), (2, "lake".into()), (10, "river".into())]
        );
        assert_eq!(bodies["bodies"][1]["level_m"], 8.0);
        let input: Value = serde_json::from_slice(
            &fs::read(Path::new(m["world_input"].as_str().unwrap()).join("world.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(input["origin"], json!([0.0, 8.0]));
    }
}
