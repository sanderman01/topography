//! Training release of accepted context regions: hard links to the immutable region files
//! plus a manifest pinning their digests, votes and the region-level geographic split.
//! GLO-30 regions also get a GEBCO seabed layer, read at release time.

use super::gallery::release_candidates;
use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, gebco, io_context, read_small, write_new,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// By default, classes with at least this many accepted regions lend one to validation.
pub const VALIDATION_MIN_REGIONS: usize = 6;

fn link(from: &Path, to: &Path) -> Result<()> {
    // Region files are never rewritten, so a hard link is a safe snapshot.
    fs::hard_link(from, to)
        .or_else(|_| fs::copy(from, to).map(|_| ()))
        .map_err(|e| AcquisitionError::Invalid(format!("link {}: {e}", from.display())))
}

/// GEBCO on the region's grid; the release joins nothing, training does (see cascade.py).
fn seabed(m: &Value) -> Result<(Vec<u8>, Value)> {
    let num = |v: &Value| {
        v.as_f64()
            .ok_or_else(|| AcquisitionError::Invalid("region.json grid".into()))
    };
    let (x0, top, s) = (
        num(&m["origin"][0])?,
        num(&m["origin"][1])?,
        num(&m["spacing_m"])?,
    );
    let (w, h) = (num(&m["size"][0])?, num(&m["size"][1])?);
    let bbox = [0, 1, 2, 3].map(|i| m["wgs84_bbox"][i].as_f64().unwrap_or(0.0));
    let srs = m["crs"].as_str().unwrap_or_default();
    let scratch = io_context("seabed scratch", tempfile::tempdir())?;
    gebco::seabed(
        srs,
        [x0, top - h * s, x0 + w * s, top],
        s,
        bbox,
        scratch.path(),
    )
}

const VALIDATION_TWO_FROM: usize = 8;
/// Regions closer than this share terrain and stay on one side of the split.
const SPLIT_BUFFER_M: f64 = 5_000.0;

/// Cluster index per region: regions whose WGS84 boxes, grown by SPLIT_BUFFER_M, overlap.
fn clusters(boxes: &[[f64; 4]]) -> Vec<usize> {
    let near = |a: &[f64; 4], b: &[f64; 4]| {
        if a.iter().chain(b).any(|v| v.is_nan()) {
            return false;
        }
        let dy = SPLIT_BUFFER_M / 111_000.0;
        let dx = dy / ((a[1] + a[3]) / 2.0).to_radians().cos().max(0.1);
        !(a[2] + dx < b[0] || b[2] + dx < a[0] || a[3] + dy < b[1] || b[3] + dy < a[1])
    };
    let mut cluster: Vec<usize> = (0..boxes.len()).collect();
    // Few regions; relabel until stable.
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..boxes.len() {
            for j in 0..boxes.len() {
                if near(&boxes[i], &boxes[j]) && cluster[j] > cluster[i] {
                    cluster[j] = cluster[i];
                    changed = true;
                }
            }
        }
    }
    cluster
}

/// `min_per_class`: classes with at least this many accepted regions lend one to validation.
/// `include_unreviewed` also releases regions without a vote, marked as unreviewed.
pub fn build_release(
    root: &Path,
    output: &Path,
    min_per_class: usize,
    include_unreviewed: bool,
) -> Result<PathBuf> {
    ensure(!output.exists(), "release output already exists")?;
    let candidates = release_candidates(root, include_unreviewed)?;
    ensure(!candidates.is_empty(), "no accepted context regions")?;
    let review: BTreeMap<String, &str> = candidates
        .iter()
        .map(|(d, s)| {
            (
                d.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                *s,
            )
        })
        .collect();
    let dirs: Vec<PathBuf> = candidates.into_iter().map(|(d, _)| d).collect();
    let mut regions = Vec::new();
    for dir in &dirs {
        let bytes = read_small(&dir.join("region.json"))?;
        let manifest: Value = serde_json::from_slice(&bytes)?;
        regions.push((dir, digest(&bytes), manifest));
    }
    // Validation regions per well-covered class, chosen by a stable hash of their ids:
    // two where a class has VALIDATION_TWO_FROM or more regions, so class gates can test
    // it on more than one place (docs/plans/class-conditioning-v1.md). Regions within
    // SPLIT_BUFFER_M of each other form a cluster that stays on one side of the split:
    // isolated regions are preferred, and a chosen clustered one takes its cluster along.
    ensure(min_per_class >= 2, "min-per-class must be at least 2")?;
    let ids: Vec<&str> = regions
        .iter()
        .map(|(_, _, m)| m["id"].as_str().unwrap_or_default())
        .collect();
    let boxes: Vec<[f64; 4]> = regions
        .iter()
        .map(|(_, _, m)| [0, 1, 2, 3].map(|i| m["wgs84_bbox"][i].as_f64().unwrap_or(f64::NAN)))
        .collect();
    let cluster = clusters(&boxes);
    let mut by_class: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, (_, _, m)) in regions.iter().enumerate() {
        by_class
            .entry(m["class_id"].as_i64().unwrap_or(0))
            .or_default()
            .push(i);
    }
    let mut validation: Vec<&str> = Vec::new();
    for members in by_class.values().filter(|m| m.len() >= min_per_class) {
        let take = if members.len() >= VALIDATION_TWO_FROM {
            2
        } else {
            1
        };
        let mut sorted = members.clone();
        let alone = |i: &usize| cluster.iter().filter(|c| **c == cluster[*i]).count() == 1;
        sorted.sort_by_key(|i| (!alone(i), digest(ids[*i].as_bytes())));
        for i in sorted.into_iter().take(take) {
            for (j, c) in cluster.iter().enumerate() {
                if *c == cluster[i] && !validation.contains(&ids[j]) {
                    validation.push(ids[j]);
                }
            }
        }
    }

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create release parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage context release", tempfile::tempdir_in(parent))?;
    let mut entries = Vec::new();
    for (dir, sha256, m) in &regions {
        let id = m["id"].as_str().unwrap_or_default();
        let target = stage.path().join("regions").join(id);
        io_context("create release region", fs::create_dir_all(&target))?;
        let mut layers = serde_json::Map::new();
        let files = m["files"].as_object().cloned().unwrap_or_default();
        for (name, expected) in files.iter().filter(|(k, _)| *k != "overview.png") {
            let name = name.as_str();
            let expected = expected.as_str().unwrap_or_default();
            let bytes = io_context(format!("read {name}"), fs::read(dir.join(name)))?;
            ensure(
                digest(&bytes) == expected,
                format!("{id}/{name} does not match region.json"),
            )?;
            link(&dir.join(name), &target.join(name))?;
            let dtype = if name.ends_with(".f32") {
                "float32_le"
            } else {
                "uint8"
            };
            layers.insert(
                name.trim_end_matches(".f32").trim_end_matches(".u8").into(),
                json!({"path": format!("regions/{id}/{name}"), "sha256": expected, "dtype": dtype}),
            );
        }
        link(&dir.join("region.json"), &target.join("region.json"))?;
        let mut seabed_source = Value::Null;
        // Early GLO-30 regions predate the "source" field.
        if m["source"] != "lidar_1m_average" {
            let (bytes, details) = seabed(m)?;
            write_new(&target.join("seabed.f32"), &bytes)?;
            layers.insert(
                "seabed".into(),
                json!({"path": format!("regions/{id}/seabed.f32"), "sha256": digest(&bytes), "dtype": "float32_le"}),
            );
            seabed_source = details;
        }
        let split = if validation.contains(&id) {
            "validation"
        } else {
            "train"
        };
        entries.push(
            json!({"id": id, "class_id": m["class_id"], "reference": m["reference"],
            "split": split, "review": review.get(id).copied().unwrap_or("yes"), "size": m["size"], "spacing_m": m["spacing_m"], "crs": m["crs"],
            "origin": m["origin"], "region_json_sha256": sha256, "layers": layers,
            "seabed_source": seabed_source}),
        );
    }
    let decisions = read_small(&root.join("decisions.jsonl"))?;
    let manifest = json!({
        "schema_version": 1,
        "kind": "context_training_release",
        "regions_root": root.canonicalize().map(|p| p.display().to_string()).unwrap_or_default(),
        "decisions_sha256": digest(&decisions),
        "unreviewed_included": include_unreviewed,
        "split": format!("whole regions; one validation region per class with {min_per_class}+ accepted regions, two with {VALIDATION_TWO_FROM}+; regions within {SPLIT_BUFFER_M} m of each other stay together"),
        "attribution": [
            "produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved",
            gebco::ATTRIBUTION, "ESA WorldCover 2021 (CC BY 4.0)", "GLIMS Glacier Database", "© OpenStreetMap contributors, ODbL"],
        "regions": entries,
    });
    write_new(
        &stage.path().join("release.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish context release", fs::rename(stage.path(), output))?;
    Ok(output.join("release.json"))
}
