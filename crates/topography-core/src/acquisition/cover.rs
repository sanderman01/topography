//! Game cover masks (`game_cover_v1`) for one reviewed crop from public land cover.
//! Technical layers only; never writes curation or creates a vote.

use super::{
    AcquisitionError, Result, digest, ensure, http,
    raster::{command, run},
    read_small,
};
use crate::{
    preparation::automatic,
    review::{self, bundle},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const POLICY: &str = "game_cover_v1";
const DEFAULT_POLICY: &str = include_str!("../../../../configs/cover-game-v1.toml");
const ABSENT: u8 = 1;
const PRESENT: u8 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    policy: String,
    worldcover: WorldCover,
    glims: Glims,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldCover {
    url_template: String,
    observed_at: String,
    snow_ice_present: Vec<u8>,
    vegetation_present: Vec<u8>,
    license: String,
    citation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Glims {
    endpoint: String,
    type_name: String,
    line_types: Vec<String>,
    ignore_before_year: i64,
    license: String,
    citation: String,
}

/// Explicit path, else `TOPOGRAPHY_COVER_POLICY`, else the built-in policy.
fn load_policy(path: Option<&Path>) -> Result<(Vec<u8>, Policy)> {
    let path = path
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("TOPOGRAPHY_COVER_POLICY").map(PathBuf::from));
    let bytes = match path {
        Some(path) => read_small(&path)?,
        None => DEFAULT_POLICY.as_bytes().to_vec(),
    };
    let policy: Policy = toml::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| AcquisitionError::Invalid("cover policy must be UTF-8".into()))?,
    )
    .map_err(|e| AcquisitionError::Invalid(format!("invalid cover policy: {e}")))?;
    let https = |url: &str| url.starts_with("https://") && !url.contains(['?', '#', ' ']);
    let (w, g) = (&policy.worldcover, &policy.glims);
    ensure(
        policy.schema_version == 1 && policy.policy == POLICY,
        "cover policy must be schema 1, game_cover_v1",
    )?;
    ensure(
        https(&w.url_template)
            && w.url_template.contains("{tile}")
            && !w.observed_at.trim().is_empty()
            && !w.snow_ice_present.is_empty()
            && https(&g.endpoint)
            && !g.line_types.is_empty()
            && !g.type_name.is_empty()
            && g.type_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b)),
        "invalid cover policy sources",
    )?;
    ensure(
        [&w.license, &w.citation, &g.license, &g.citation]
            .iter()
            .all(|text| !text.trim().is_empty()),
        "cover policy licenses and citations must be nonempty",
    )?;
    Ok((bytes, policy))
}

/// The review crop grid as GDAL reported it.
struct Grid {
    width: u32,
    height: u32,
    transform: [f64; 6],
    wkt: String,
}

impl Grid {
    fn cells(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Extent as [xmin, ymin, xmax, ymax].
    fn extent(&self) -> [f64; 4] {
        let t = &self.transform;
        [
            t[0],
            t[3] + f64::from(self.height) * t[5],
            t[0] + f64::from(self.width) * t[1],
            t[3],
        ]
    }
}

/// GDAL command with bounded HTTP behaviour for public COG reads.
fn gdal(program: &str) -> Command {
    let mut c = command(program);
    c.env("GDAL_HTTP_MAX_RETRY", "3")
        .env("GDAL_HTTP_RETRY_DELAY", "2")
        .env("GDAL_HTTP_TIMEOUT", "60")
        .env("GDAL_HTTP_CONNECTTIMEOUT", "15")
        .env("CPL_VSIL_CURL_ALLOWED_EXTENSIONS", ".tif,.TIF,.tiff")
        .env("CPL_VSIL_CURL_USE_HEAD", "YES");
    c
}

fn wgs84_bbox(review: &Path) -> Result<[f64; 4]> {
    let elevation = review
        .parent()
        .unwrap_or(Path::new("."))
        .join("elevation.tif");
    let info: Value = serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-nomd"])
        .arg(&elevation))?)?;
    let ring = info["wgs84Extent"]["coordinates"][0]
        .as_array()
        .filter(|r| r.len() >= 4)
        .ok_or_else(|| AcquisitionError::Invalid("review crop has no WGS84 extent".into()))?;
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for point in ring {
        let (lon, lat) = (
            point[0].as_f64().unwrap_or(f64::NAN),
            point[1].as_f64().unwrap_or(f64::NAN),
        );
        ensure(lon.is_finite() && lat.is_finite(), "invalid WGS84 extent")?;
        bbox = [
            bbox[0].min(lon),
            bbox[1].min(lat),
            bbox[2].max(lon),
            bbox[3].max(lat),
        ];
    }
    ensure(
        bbox[2] - bbox[0] < 1.0 && bbox[3] - bbox[1] < 1.0,
        "review crop WGS84 extent exceeds one degree or crosses the antimeridian",
    )?;
    Ok(bbox)
}

fn coord(v: f64) -> String {
    format!("{v:.7}")
}

fn worldcover_tiles(bbox: [f64; 4]) -> Vec<String> {
    let step = |v: f64| (v / 3.0).floor() as i64 * 3;
    let mut tiles = Vec::new();
    for lat in (step(bbox[1])..=step(bbox[3])).step_by(3) {
        for lon in (step(bbox[0])..=step(bbox[2])).step_by(3) {
            tiles.push(format!(
                "{}{:02}{}{:03}",
                if lat < 0 { 'S' } else { 'N' },
                lat.abs(),
                if lon < 0 { 'W' } else { 'E' },
                lon.abs()
            ));
        }
    }
    tiles
}

/// WorldCover classes on the crop grid; 0 where no tile exists (outside coverage).
fn worldcover(
    p: &WorldCover,
    bbox: [f64; 4],
    grid: &Grid,
    scratch: &Path,
) -> Result<(Vec<u8>, Value)> {
    let (mut urls, mut missing) = (Vec::new(), Vec::new());
    for tile in worldcover_tiles(bbox) {
        let url = p.url_template.replace("{tile}", &tile);
        if http::exists(&url)? {
            urls.push(url);
        } else {
            missing.push(url);
        }
    }
    let details = json!({"urls":urls,"outside_coverage":missing,"observed_at":p.observed_at,
        "license":p.license,"citation":p.citation});
    if urls.is_empty() {
        return Ok((vec![0; grid.cells()], details));
    }
    let vrt = scratch.join("worldcover.vrt");
    run(gdal("gdalbuildvrt")
        .arg("-q")
        .arg(&vrt)
        .args(urls.iter().map(|u| format!("/vsicurl/{u}"))))?;
    let placed = scratch.join("worldcover.tif");
    run(gdal("gdalwarp")
        .args(["-q", "-r", "near", "-et", "0", "-ot", "Byte"])
        .args([
            "-wo",
            "INIT_DEST=0",
            "-dstnodata",
            "None",
            "-t_srs",
            &grid.wkt,
            "-te",
        ])
        .args(grid.extent().map(|v| v.to_string()))
        .args(["-ts", &grid.width.to_string(), &grid.height.to_string()])
        .args(["-of", "GTiff"])
        .arg(&vrt)
        .arg(&placed))?;
    Ok((
        review::band(&placed, scratch, "worldcover", grid.cells(), false)?,
        details,
    ))
}

/// Burn the most recent outline of each glacier (1900 or later) onto the crop grid.
fn glims(p: &Glims, bbox: [f64; 4], grid: &Grid, scratch: &Path) -> Result<(Vec<u8>, Value)> {
    let [west, south, east, north] = bbox;
    let url = format!(
        "{}?service=WFS&version=2.0.0&request=GetFeature&typeNames={}&outputFormat=application/json&srsName=EPSG:4326&bbox={},{},{},{},urn:ogc:def:crs:EPSG::4326",
        p.endpoint,
        p.type_name,
        coord(south),
        coord(west),
        coord(north),
        coord(east)
    );
    let body: Value = serde_json::from_slice(&http::query(&url)?)?;
    let features = body["features"]
        .as_array()
        .filter(|_| body["type"] == "FeatureCollection")
        .ok_or_else(|| {
            AcquisitionError::Invalid("GLIMS response is not a FeatureCollection".into())
        })?;
    if let Some(matched) = body["numberMatched"].as_u64() {
        ensure(
            matched as usize == features.len(),
            "GLIMS response is truncated",
        )?;
    }
    let mut latest: BTreeMap<&str, (&str, i64, &Value)> = BTreeMap::new();
    let mut ignored = 0;
    for feature in features {
        let props = &feature["properties"];
        if !p
            .line_types
            .iter()
            .any(|t| props["line_type"].as_str() == Some(t))
        {
            continue;
        }
        let (Some(id), Some(date)) = (
            props["glac_id"].as_str().filter(|v| !v.is_empty()),
            props["src_date"].as_str().and_then(|d| d.get(..10)),
        ) else {
            ignored += 1;
            continue;
        };
        if date[..4].parse::<i64>().unwrap_or(0) < p.ignore_before_year {
            ignored += 1;
            continue;
        }
        ensure(
            matches!(
                feature["geometry"]["type"].as_str(),
                Some("Polygon" | "MultiPolygon")
            ),
            "GLIMS outline geometry must be a polygon",
        )?;
        // GeoServer declares a lat/lon URN but writes lon/lat; verify rather than assume.
        let mut point = &feature["geometry"]["coordinates"];
        while point[0].is_array() {
            point = &point[0];
        }
        ensure(
            point[0]
                .as_f64()
                .zip(point[1].as_f64())
                .is_some_and(|(lon, lat)| {
                    (west - 1.0..=east + 1.0).contains(&lon)
                        && (south - 1.0..=north + 1.0).contains(&lat)
                }),
            "GLIMS coordinates are not lon/lat near the requested bbox",
        )?;
        let analysis = props["anlys_id"].as_i64().unwrap_or(0);
        if latest
            .get(id)
            .is_none_or(|(d, a, _)| (date, analysis) > (*d, *a))
        {
            latest.insert(id, (date, analysis, feature));
        }
    }
    let raster = scratch.join("glims.tif");
    let e = grid.extent();
    run(command("gdal_create")
        .args([
            "-q", "-of", "GTiff", "-bands", "1", "-ot", "Byte", "-burn", "0",
        ])
        .args([
            "-outsize",
            &grid.width.to_string(),
            &grid.height.to_string(),
        ])
        .args(["-a_srs", &grid.wkt, "-a_ullr"])
        .args([e[0], e[3], e[2], e[1]].map(|v| v.to_string()))
        .arg(&raster))?;
    if !latest.is_empty() {
        let vector = scratch.join("glims.geojson");
        let features: Vec<Value> = latest
            .values()
            .map(|(_, _, f)| json!({"type":"Feature","properties":{},"geometry":f["geometry"]}))
            .collect();
        fs::write(
            &vector,
            serde_json::to_vec(&json!({"type":"FeatureCollection","features":features}))?,
        )
        .map_err(|e| AcquisitionError::Io {
            context: "write GLIMS outlines".into(),
            source: e,
        })?;
        run(command("gdal_rasterize")
            .args(["-q", "-burn", "1"])
            .arg(&vector)
            .arg(&raster))?;
    }
    let glaciers: Vec<Value> = latest
        .iter()
        .map(|(id, (date, analysis, _))| json!({"glac_id":id,"anlys_id":analysis,"src_date":date}))
        .collect();
    Ok((
        review::band(&raster, scratch, "glims", grid.cells(), false)?,
        json!({"url":url,"glaciers":glaciers,"ignored_outlines":ignored,"license":p.license,"citation":p.citation}),
    ))
}

/// Generated crop-grid cover layers in a caller-owned directory.
pub(crate) struct Generated {
    pub(crate) manifest: PathBuf,
    pub(crate) note: String,
    pub(crate) observed_at: String,
}

/// Write `snow-ice.tif`, `vegetation.tif` and `cover.json` for one verified crop.
/// Unreachable sources fail; a crop outside WorldCover coverage is absent.
pub(crate) fn generate(bundle: &bundle::Bundle, review: &Path, output: &Path) -> Result<Generated> {
    let (policy_bytes, policy) = load_policy(None)?;
    generate_with(bundle, review, output, &policy_bytes, &policy)
}

fn generate_with(
    bundle: &bundle::Bundle,
    review: &Path,
    output: &Path,
    policy_bytes: &[u8],
    policy: &Policy,
) -> Result<Generated> {
    let info = &bundle.inspection;
    let [width, height]: [u32; 2] = serde_json::from_value(info["size"].clone())?;
    let grid = Grid {
        width,
        height,
        transform: serde_json::from_value(info["geoTransform"].clone())?,
        wkt: info["coordinateSystem"]["wkt"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AcquisitionError::Invalid("review CRS WKT missing".into()))?
            .into(),
    };
    let scratch = super::io_context("create cover scratch", tempfile::tempdir())?;
    let bbox = wgs84_bbox(review)?;
    let (classes, wc) = worldcover(&policy.worldcover, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("WorldCover unavailable: {e}")))?;
    let (glaciers, gl) = glims(&policy.glims, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("GLIMS unavailable: {e}")))?;
    let w = &policy.worldcover;
    let snow: Vec<u8> = classes
        .iter()
        .zip(&glaciers)
        .map(|(c, g)| {
            if w.snow_ice_present.contains(c) || *g == 1 {
                PRESENT
            } else {
                ABSENT
            }
        })
        .collect();
    let vegetation: Vec<u8> = classes
        .iter()
        .map(|c| {
            if w.vegetation_present.contains(c) {
                PRESENT
            } else {
                ABSENT
            }
        })
        .collect();
    for (name, values) in [("snow-ice", &snow), ("vegetation", &vegetation)] {
        automatic::write_crop_mask(&output.join(format!("{name}.tif")), info, values)?;
    }
    let count = |v: &[u8]| v.iter().filter(|x| **x == PRESENT).count();
    let mut note = format!(
        "{POLICY}: snow/ice is WorldCover {} class 70 or inside the latest GLIMS outline per glacier ({} glaciers); vegetation is WorldCover tree, shrub, grass, crop, wetland, mangrove and moss classes; everything else is absent. Nearest neighbour on the crop grid. Game judgment layers, not measurements.",
        w.observed_at,
        gl["glaciers"].as_array().map_or(0, Vec::len)
    );
    if wc["urls"].as_array().is_some_and(Vec::is_empty) {
        note.push_str(" The crop lies outside WorldCover coverage; recorded as absent (judgment).");
    }
    let manifest = json!({
        "schema_version": 1,
        "kind": "game_cover",
        "policy": POLICY,
        "policy_sha256": digest(policy_bytes),
        "policy_text": String::from_utf8_lossy(policy_bytes),
        "raw_receipt_sha256": bundle.manifest.raw_receipt_sha256,
        "review_sha256": digest(&read_small(review)?),
        "window_cells": bundle.manifest.window_cells,
        "wgs84_bbox": bbox,
        "sources": {"worldcover": wc, "glims": gl},
        "counts": {"cells": grid.cells(), "snow_ice_present": count(&snow), "vegetation_present": count(&vegetation)},
        "note": note,
        "admission": "technical layers only; no curation, vote or acceptance",
    });
    let path = output.join("cover.json");
    super::write_new(&path, &serde_json::to_vec_pretty(&manifest)?)?;
    Ok(Generated {
        manifest: path,
        note,
        observed_at: w.observed_at.clone(),
    })
}

/// Write game cover for one reviewed crop into a new directory (inspection aid).
pub fn draft(
    raw: &Path,
    review: &Path,
    output: &Path,
    policy_path: Option<&Path>,
) -> Result<String> {
    ensure(
        !output.exists(),
        "cover output already exists; choose a new directory",
    )?;
    let (policy_bytes, policy) = load_policy(policy_path)?;
    let bundle = bundle::verify(raw, review)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    super::io_context("create cover parent", fs::create_dir_all(parent))?;
    let stage = super::io_context("stage cover draft", tempfile::tempdir_in(parent))?;
    let generated = generate_with(&bundle, review, stage.path(), &policy_bytes, &policy)?;
    let body: Value = super::read_json(&generated.manifest)?;
    super::io_context("publish cover draft", fs::rename(stage.path(), output))?;
    Ok(format!(
        "{}\nSnow/ice present {} and vegetation present {} of {} cells.\n{}",
        output.join("cover.json").display(),
        body["counts"]["snow_ice_present"],
        body["counts"]["vegetation_present"],
        body["counts"]["cells"],
        generated.note
    ))
}
