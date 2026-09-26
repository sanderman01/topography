//! OpenStreetMap water from Geofabrik regional extracts, without per-crop API calls.
//! Each region's extract is downloaded once and reduced to a GeoPackage of water
//! features; crops read their bounding box from that local file.

use super::{
    AcquisitionError, Result, ensure, hash_file, http, io_context,
    raster::{command, run},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

static CACHE: OnceLock<PathBuf> = OnceLock::new();

const INDEX_MAX_AGE: Duration = Duration::from_secs(30 * 86_400);
// Game layers: a twice-yearly refresh is plenty.
const EXTRACT_MAX_AGE: Duration = Duration::from_secs(180 * 86_400);
const MAX_EXTRACT_BYTES: u64 = 4 << 30;
const WHERE: &str = "natural IN ('water','mud') OR water IS NOT NULL \
    OR waterway IN ('riverbank','river','stream','canal') OR wetland = 'tidalflat'";
// GDAL OSM driver layout: only the tags the water rules read, on lines and areas.
const OSMCONF: &str = "[general]
closed_ways_are_polygons=natural,water,wetland,waterway=riverbank,landuse
[points]
osm_id=no
attributes=
other_tags=no
[lines]
osm_id=yes
attributes=natural,water,waterway,wetland,width
other_tags=no
[multipolygons]
osm_id=yes
attributes=natural,water,waterway,wetland,width
other_tags=no
[multilinestrings]
osm_id=no
attributes=
other_tags=no
[other_relations]
osm_id=no
attributes=
other_tags=no
";

/// Keep regional extracts in the runtime's `<cache>/osm-extracts`. Commands that
/// build water call this; unset, extracts go to `data/cache/osm-extracts`.
pub fn use_runtime(runtime: &Path) -> Result<()> {
    let cache = crate::config::inspect_runtime(runtime)?
        .paths
        .into_iter()
        .find(|(key, _)| *key == "cache")
        .ok_or_else(|| AcquisitionError::Invalid("runtime cache path missing".into()))?
        .1;
    let _ = CACHE.set(cache.join("osm-extracts"));
    Ok(())
}

fn cache_dir() -> PathBuf {
    CACHE
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("data/cache/osm-extracts"))
}

fn fresh(path: &Path, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < max_age)
}

/// Write via a temporary sibling so readers never see partial files.
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("partial");
    io_context("write extract cache", fs::write(&temp, bytes))?;
    io_context("publish extract cache", fs::rename(&temp, path))
}

type Ring = Vec<[f64; 2]>;

fn rings(geometry: &Value) -> Vec<Vec<Ring>> {
    let polygon = |p: &Value| -> Vec<Ring> {
        p.as_array()
            .into_iter()
            .flatten()
            .map(|ring| {
                ring.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| Some([c[0].as_f64()?, c[1].as_f64()?]))
                    .collect()
            })
            .collect()
    };
    match geometry["type"].as_str() {
        Some("Polygon") => vec![polygon(&geometry["coordinates"])],
        Some("MultiPolygon") => geometry["coordinates"]
            .as_array()
            .into_iter()
            .flatten()
            .map(polygon)
            .collect(),
        _ => Vec::new(),
    }
}

/// Even-odd point test over all rings of one polygon.
fn inside(polygon: &[Ring], [x, y]: [f64; 2]) -> bool {
    let mut odd = false;
    for ring in polygon {
        for pair in ring.windows(2) {
            let ([x1, y1], [x2, y2]) = (pair[0], pair[1]);
            if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
                odd = !odd;
            }
        }
    }
    odd
}

fn area(polygon: &[Ring]) -> f64 {
    polygon.first().map_or(0.0, |ring| {
        ring.windows(2)
            .map(|p| p[0][0] * p[1][1] - p[1][0] * p[0][1])
            .sum::<f64>()
            .abs()
            / 2.0
    })
}

/// The smallest Geofabrik region whose outline contains the whole bbox.
fn region(index: &Value, [west, south, east, north]: [f64; 4]) -> Result<(String, String)> {
    let corners = [[west, south], [west, north], [east, south], [east, north]];
    index["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let pbf = f["properties"]["urls"]["pbf"].as_str()?;
            let id = f["properties"]["id"].as_str()?;
            rings(&f["geometry"])
                .iter()
                .filter(|p| corners.iter().all(|c| inside(p, *c)))
                .map(|p| area(p))
                .reduce(f64::min)
                .map(|a| (a, id.to_owned(), pbf.to_owned()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id, pbf)| (id, pbf))
        .ok_or_else(|| AcquisitionError::Invalid("no Geofabrik region contains this crop".into()))
}

/// Download the region's extract and keep its water features as a GeoPackage.
fn build(dir: &Path, id: &str, pbf_url: &str, user_agent: &str) -> Result<Value> {
    let url = http::resolve(pbf_url)?;
    let identity = http::identify(&url, MAX_EXTRACT_BYTES)?;
    let name = id.replace('/', "-");
    let pbf = dir.join(format!("{name}.osm.pbf"));
    http::download(&identity, &dir.join("staging"), &pbf)?;
    // Verified chunks are only needed to resume an interrupted download.
    let _ = fs::remove_dir_all(dir.join("staging"));
    let (sha256, bytes) = hash_file(&pbf)?;
    let conf = dir.join("osmconf-water.ini");
    replace(&conf, OSMCONF.as_bytes())?;
    let gpkg = dir.join(format!("{name}.water.gpkg"));
    let temp = dir.join(format!("{name}.water.partial.gpkg"));
    let _ = fs::remove_file(&temp);
    let result = run(command("ogr2ogr")
        // The node index spills to disk here, not to a small tmpfs /tmp.
        .env("CPL_TMPDIR", dir)
        .env("OSM_CONFIG_FILE", &conf)
        .env("OSM_MAX_TMPFILE_SIZE", "2048")
        .args([
            "-q",
            "-f",
            "GPKG",
            "-nlt",
            "PROMOTE_TO_MULTI",
            "-where",
            WHERE,
        ])
        .arg(&temp)
        .arg(&pbf)
        .args(["multipolygons", "lines"]));
    let _ = fs::remove_file(&pbf);
    result?;
    io_context("publish water extract", fs::rename(&temp, &gpkg))?;
    let provenance = json!({"region": id, "url": url, "etag": identity.etag,
        "pbf_sha256": sha256, "pbf_bytes": bytes, "user_agent": user_agent, "where": WHERE});
    replace(
        &dir.join(format!("{name}.json")),
        &serde_json::to_vec_pretty(&provenance)?,
    )?;
    Ok(provenance)
}

/// Water areas and lines (GeoJSON feature collections in lon/lat) intersecting `bbox`,
/// plus provenance of the extract they came from.
pub(crate) fn water(
    index_url: &str,
    user_agent: &str,
    bbox: [f64; 4],
    scratch: &Path,
) -> Result<(Value, Value, Value)> {
    let dir = cache_dir();
    io_context("create extract cache", fs::create_dir_all(&dir))?;
    let index_path = dir.join("index-v1.json");
    if !fresh(&index_path, INDEX_MAX_AGE) {
        replace(&index_path, &http::query_large(index_url, user_agent, 300)?)?;
    }
    let index: Value =
        serde_json::from_slice(&io_context("read Geofabrik index", fs::read(&index_path))?)?;
    let (id, pbf_url) = region(&index, bbox)?;
    let name = id.replace('/', "-");
    let gpkg = dir.join(format!("{name}.water.gpkg"));
    let meta = dir.join(format!("{name}.json"));
    let provenance = if gpkg.is_file() && meta.is_file() && fresh(&gpkg, EXTRACT_MAX_AGE) {
        serde_json::from_slice(&io_context("read extract provenance", fs::read(&meta))?)?
    } else {
        build(&dir, &id, &pbf_url, user_agent)?
    };
    let [west, south, east, north] = bbox.map(|v| v.to_string());
    let mut layers = Vec::new();
    for layer in ["multipolygons", "lines"] {
        let out = scratch.join(format!("osm-{layer}.geojson"));
        run(command("ogr2ogr")
            .args(["-q", "-f", "GeoJSON", "-spat"])
            .args([&west, &south, &east, &north])
            .arg(&out)
            .arg(&gpkg)
            .arg(layer))?;
        let collection: Value =
            serde_json::from_slice(&io_context("read extract water", fs::read(&out))?)?;
        ensure(
            collection["features"].is_array(),
            "extract water is not a feature collection",
        )?;
        layers.push(collection);
    }
    let lines = layers.pop().unwrap_or_default();
    let areas = layers.pop().unwrap_or_default();
    Ok((areas, lines, provenance))
}
