//! Game cover masks (`game_cover_v6`) for one reviewed crop from public land cover
//! and OpenStreetMap water, plus the WorldCover class layer. Technical layers only; never writes curation or creates a vote.

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

pub(crate) const POLICY: &str = "game_cover_v6";
const DEFAULT_POLICY: &str = include_str!("../../../../configs/cover-game-v6.toml");
/// ESA WorldCover v200 class codes; 0 marks cells outside WorldCover coverage.
pub(crate) const LAND_COVER_CLASSES: [u8; 12] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 95, 100];
const ABSENT: u8 = 1;
const PRESENT: u8 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    policy: String,
    worldcover: WorldCover,
    glims: Glims,
    osm: Osm,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldCover {
    url_template: String,
    observed_at: String,
    water_present: Vec<u8>,
    water_smooth_m: f64,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Osm {
    /// "geofabrik" (regional extracts) or "overpass" (per-crop API queries).
    source: String,
    index_url: String,
    endpoint: String,
    user_agent: String,
    timeout_s: u64,
    river_width_m: f64,
    canal_width_m: f64,
    stream_width_m: f64,
    license: String,
    attribution: String,
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
    let (w, g, o) = (&policy.worldcover, &policy.glims, &policy.osm);
    ensure(
        policy.schema_version == 2 && policy.policy == POLICY,
        "cover policy must be schema 2, game_cover_v6",
    )?;
    ensure(
        https(&w.url_template)
            && w.url_template.contains("{tile}")
            && !w.observed_at.trim().is_empty()
            && !w.snow_ice_present.is_empty()
            && !w.water_present.is_empty()
            && w.water_smooth_m.is_finite()
            && (0.0..=50.0).contains(&w.water_smooth_m)
            && https(&g.endpoint)
            && https(&o.endpoint)
            && (1..=600).contains(&o.timeout_s)
            && [o.river_width_m, o.canal_width_m, o.stream_width_m]
                .iter()
                .all(|w| w.is_finite() && *w > 0.0 && *w <= 1000.0)
            && !o.user_agent.trim().is_empty()
            && o.user_agent
                .bytes()
                .all(|b| b.is_ascii_graphic() || b == b' ')
            && !g.line_types.is_empty()
            && !g.type_name.is_empty()
            && g.type_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b)),
        "invalid cover policy sources",
    )?;
    ensure(
        [
            &w.license,
            &w.citation,
            &g.license,
            &g.citation,
            &o.license,
            &o.attribution,
        ]
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

/// WorldCover water on the crop grid with its 10 m steps rounded: a Gaussian blur of
/// `sigma_m` (edges replicated) thresholded at one half.
fn smooth_water(water: &[bool], grid: &Grid, sigma_m: f64) -> Vec<bool> {
    let spacing = grid.transform[1].abs();
    if sigma_m <= 0.0 || spacing <= 0.0 || !water.contains(&true) {
        return water.to_vec();
    }
    let sigma = sigma_m / spacing;
    let r = (3.0 * sigma) as isize;
    let kernel: Vec<f32> = (-r..=r)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .map(|k| k as f32)
        .collect();
    let total: f32 = kernel.iter().sum();
    let (w, h) = (grid.width as usize, grid.height as usize);
    let blur = |src: &[f32], dst: &mut [f32], horizontal: bool| {
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let d = k as isize - r;
                    let (sx, sy) = if horizontal {
                        ((x as isize + d).clamp(0, w as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + d).clamp(0, h as isize - 1) as usize)
                    };
                    sum += weight * src[sy * w + sx];
                }
                dst[y * w + x] = sum / total;
            }
        }
    };
    let input: Vec<f32> = water.iter().map(|v| f32::from(u8::from(*v))).collect();
    let mut pass = vec![0.0; input.len()];
    let mut output = vec![0.0; input.len()];
    blur(&input, &mut pass, true);
    blur(&pass, &mut output, false);
    output.iter().map(|v| *v > 0.5).collect()
}

/// GDAL command with bounded HTTP behaviour for public COG reads.
pub(super) fn gdal(program: &str) -> Command {
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
    blank(grid, &raster)?;
    let geometries: Vec<Value> = latest
        .values()
        .map(|(_, _, f)| f["geometry"].clone())
        .collect();
    burn(&geometries, &scratch.join("glims.geojson"), false, &raster)?;
    let glaciers: Vec<Value> = latest
        .iter()
        .map(|(id, (date, analysis, _))| json!({"glac_id":id,"anlys_id":analysis,"src_date":date}))
        .collect();
    Ok((
        review::band(&raster, scratch, "glims", grid.cells(), false)?,
        json!({"url":url,"glaciers":glaciers,"ignored_outlines":ignored,"license":p.license,"citation":p.citation}),
    ))
}

/// Zero Byte raster on the crop grid.
fn blank(grid: &Grid, path: &Path) -> Result<()> {
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
        .arg(path))?;
    Ok(())
}

/// Burn lon/lat geometries as 1; gdal_rasterize reprojects onto the raster CRS.
fn burn(geometries: &[Value], vector: &Path, all_touched: bool, raster: &Path) -> Result<()> {
    if geometries.is_empty() {
        return Ok(());
    }
    let features: Vec<Value> = geometries
        .iter()
        .map(|g| json!({"type":"Feature","properties":{},"geometry":g}))
        .collect();
    fs::write(
        vector,
        serde_json::to_vec(&json!({"type":"FeatureCollection","features":features}))?,
    )
    .map_err(|e| AcquisitionError::Io {
        context: "write cover vectors".into(),
        source: e,
    })?;
    let mut c = command("gdal_rasterize");
    c.args(["-q", "-burn", "1"]);
    if all_touched {
        c.arg("-at");
    }
    run(c.arg(vector).arg(raster))?;
    Ok(())
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

type Point = [f64; 2];

fn points(geometry: &Value) -> Vec<Point> {
    geometry
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some([p["lon"].as_f64()?, p["lat"].as_f64()?]))
        .filter(|[lon, lat]| lon.is_finite() && lat.is_finite())
        .collect()
}

fn closed(ring: &[Point]) -> bool {
    ring.len() >= 4 && ring.first() == ring.last()
}

/// Join multipolygon member ways end to end into closed rings; open chains are dropped.
fn rings(mut pending: Vec<Vec<Point>>) -> (Vec<Vec<Point>>, usize) {
    pending.retain(|w| w.len() >= 2);
    let (mut done, mut open) = (Vec::new(), 0);
    while let Some(mut ring) = pending.pop() {
        let mut reversed = false;
        loop {
            if closed(&ring) {
                done.push(ring);
                break;
            }
            let end = *ring.last().expect("nonempty ring");
            if let Some(i) = pending
                .iter()
                .position(|w| w.first() == Some(&end) || w.last() == Some(&end))
            {
                let mut next = pending.swap_remove(i);
                if next.first() != Some(&end) {
                    next.reverse();
                }
                ring.extend_from_slice(&next[1..]);
            } else if !reversed {
                ring.reverse();
                reversed = true;
            } else {
                open += 1;
                break;
            }
        }
    }
    (done, open)
}

/// Rectangles around each segment, extended by half the width to cover joints.
fn buffered(line: &[Point], width_m: f64) -> Vec<Value> {
    let half = width_m / 2.0;
    line.windows(2)
        .filter_map(|pair| {
            let [a, b] = [pair[0], pair[1]];
            let kx = 111_320.0 * ((a[1] + b[1]) / 2.0).to_radians().cos();
            let ky = 110_540.0;
            let (dx, dy) = ((b[0] - a[0]) * kx, (b[1] - a[1]) * ky);
            let length = dx.hypot(dy);
            if length == 0.0 || kx <= 0.0 {
                return None;
            }
            let (ux, uy) = (dx / length * half, dy / length * half);
            let corner = |p: Point, along: f64, side: f64| {
                json!([
                    p[0] + (ux * along - uy * side) / kx,
                    p[1] + (uy * along + ux * side) / ky
                ])
            };
            let ring = [
                corner(a, -1.0, 1.0),
                corner(b, 1.0, 1.0),
                corner(b, 1.0, -1.0),
                corner(a, -1.0, -1.0),
                corner(a, -1.0, 1.0),
            ];
            Some(json!({"type":"Polygon","coordinates":[ring]}))
        })
        .collect()
}

/// Numeric `width` tag in metres (optionally suffixed "m"); anything else uses the default.
fn width(tag: &Value, default: f64) -> f64 {
    tag.as_str()
        .map(|v| v.trim().trim_end_matches('m').trim())
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1000.0)
        .unwrap_or(default)
}

/// Water areas, plus tidal flats: under water at high tide, which is how players see them.
fn water_area(tags: &Value) -> bool {
    tags["natural"] == "water"
        || tags.get("water").is_some()
        || tags["waterway"] == "riverbank"
        || tags["natural"] == "mud"
        || tags["wetland"] == "tidalflat"
}

/// OpenStreetMap water on the crop grid, from a Geofabrik regional extract or one
/// cached Overpass bbox query, as the policy's `source` says.
fn osm(
    p: &Osm,
    bbox: [f64; 4],
    grid: &Grid,
    scratch: &Path,
    cache: Option<&Path>,
) -> Result<(Vec<u8>, Value)> {
    let (areas, lines, sources) = match p.source.as_str() {
        "geofabrik" => extract_water(p, bbox, scratch)?,
        _ => overpass_water(p, bbox, cache)?,
    };
    let raster = scratch.join("osm.tif");
    blank(grid, &raster)?;
    burn(&areas, &scratch.join("osm-areas.geojson"), false, &raster)?;
    burn(&lines, &scratch.join("osm-lines.geojson"), true, &raster)?;
    let mut sources = sources;
    sources["default_widths_m"] =
        json!({"river":p.river_width_m,"canal":p.canal_width_m,"stream":p.stream_width_m});
    sources["license"] = json!(p.license);
    sources["attribution"] = json!(p.attribution);
    Ok((
        review::band(&raster, scratch, "osm", grid.cells(), false)?,
        sources,
    ))
}

/// Buffered line width for a river, canal or stream way; None for other lines.
fn line_width(p: &Osm, tags: &Value) -> Option<f64> {
    let default = match tags["waterway"].as_str() {
        Some("river") => p.river_width_m,
        Some("canal") => p.canal_width_m,
        Some("stream") => p.stream_width_m,
        _ => return None,
    };
    Some(width(&tags["width"], default))
}

/// Water areas and buffered lines from the local extract of the crop's region.
fn extract_water(
    p: &Osm,
    bbox: [f64; 4],
    scratch: &Path,
) -> Result<(Vec<Value>, Vec<Value>, Value)> {
    let (area_features, line_features, extract) =
        super::geofabrik::water(&p.index_url, &p.user_agent, bbox, scratch)?;
    let features = |c: &Value| c["features"].as_array().cloned().unwrap_or_default();
    let areas: Vec<Value> = features(&area_features)
        .into_iter()
        .filter(|f| water_area(&f["properties"]))
        .map(|f| f["geometry"].clone())
        .collect();
    let mut lines = Vec::new();
    let mut line_count = 0;
    for f in features(&line_features) {
        let Some(width_m) = line_width(p, &f["properties"]) else {
            continue;
        };
        let g = &f["geometry"];
        let parts: Vec<Value> = match g["type"].as_str() {
            Some("LineString") => vec![g["coordinates"].clone()],
            Some("MultiLineString") => g["coordinates"].as_array().cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        for part in parts {
            let line: Vec<Point> = part
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| Some([c[0].as_f64()?, c[1].as_f64()?]))
                .collect();
            lines.extend(buffered(&line, width_m));
        }
        line_count += 1;
    }
    let area_count = areas.len();
    Ok((
        areas,
        lines,
        json!({"source": "geofabrik", "extract": extract, "areas": area_count, "lines": line_count}),
    ))
}

/// Water areas and buffered lines from one cached Overpass bbox query.
fn overpass_water(
    p: &Osm,
    bbox: [f64; 4],
    cache: Option<&Path>,
) -> Result<(Vec<Value>, Vec<Value>, Value)> {
    let [west, south, east, north] = bbox;
    let b = format!(
        "({},{},{},{})",
        coord(south),
        coord(west),
        coord(north),
        coord(east)
    );
    let mut query = format!("[out:json][timeout:{}];(", p.timeout_s);
    for filter in [
        "[\"natural\"=\"water\"]",
        "[\"water\"]",
        "[\"waterway\"=\"riverbank\"]",
        "[\"natural\"=\"mud\"]",
        "[\"wetland\"=\"tidalflat\"]",
    ] {
        query.push_str(&format!("way{filter}{b};relation{filter}{b};"));
    }
    query.push_str(&format!(
        "way[\"waterway\"~\"^(river|stream|canal)$\"]{b};);out geom;"
    ));
    let url = format!("{}?data={}", p.endpoint, percent(&query));
    let cached = cache.map(|d| d.join(format!("overpass-{}.json", digest(url.as_bytes()))));
    let hit = cached.as_ref().filter(|path| path.is_file());
    let bytes = match hit {
        Some(path) => super::io_context("read cached Overpass response", fs::read(path))?,
        None => http::query_large(&url, &p.user_agent, p.timeout_s + 60)?,
    };
    let body: Value = serde_json::from_slice(&bytes)?;
    let elements = body["elements"]
        .as_array()
        .ok_or_else(|| AcquisitionError::Invalid("Overpass response has no elements".into()))?;
    // Overpass reports timeouts and memory errors as a remark on a 200 response.
    ensure(
        body.get("remark").is_none(),
        format!("Overpass response incomplete: {}", body["remark"]),
    )?;
    if let (None, Some(path)) = (hit, &cached) {
        super::write_new(path, &bytes)?;
    }
    let (mut areas, mut lines) = (Vec::new(), Vec::new());
    let (mut area_count, mut line_count, mut open) = (0, 0, 0);
    for element in elements {
        let tags = &element["tags"];
        match element["type"].as_str() {
            Some("way") => {
                let geometry = points(&element["geometry"]);
                if let Some(width_m) = line_width(p, tags) {
                    lines.extend(buffered(&geometry, width_m));
                    line_count += 1;
                } else if water_area(tags) && closed(&geometry) {
                    areas.push(json!({"type":"Polygon","coordinates":[geometry]}));
                    area_count += 1;
                }
            }
            Some("relation") if water_area(tags) => {
                let members = element["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| m["type"] == "way" && m["role"] != "subarea")
                    .map(|m| points(&m["geometry"]))
                    .collect();
                let (found, dropped) = rings(members);
                open += dropped;
                if !found.is_empty() {
                    // One polygon of all rings; even-odd filling leaves inner islands dry.
                    areas.push(json!({"type":"Polygon","coordinates":found}));
                    area_count += 1;
                }
            }
            _ => {}
        }
    }
    Ok((
        areas,
        lines,
        json!({"source":"overpass","endpoint":p.endpoint,"query":query,"response_sha256":digest(&bytes),
            "areas":area_count,"lines":line_count,"open_rings_ignored":open}),
    ))
}

/// Game water, snow/ice and vegetation (1 absent / 2 present) and water cell counts
/// per source (existing, WorldCover, OpenStreetMap).
struct Masks {
    water: Vec<u8>,
    snow: Vec<u8>,
    vegetation: Vec<u8>,
    sources: [usize; 3],
}

fn combine(
    w: &WorldCover,
    grid: &Grid,
    classes: &[u8],
    glaciers: &[u8],
    mapped: &[u8],
    existing_water: Option<&[u8]>,
) -> Masks {
    let worldcover_water: Vec<bool> = classes
        .iter()
        .map(|c| w.water_present.contains(c))
        .collect();
    let worldcover_water = smooth_water(&worldcover_water, grid, w.water_smooth_m);
    let present = |yes: bool| if yes { PRESENT } else { ABSENT };
    let cells = grid.cells();
    let mut m = Masks {
        water: Vec::with_capacity(cells),
        snow: Vec::with_capacity(cells),
        vegetation: Vec::with_capacity(cells),
        sources: [0; 3],
    };
    for i in 0..cells {
        let layers = [
            existing_water.is_some_and(|v| v[i] == PRESENT),
            worldcover_water[i],
            mapped[i] == 1,
        ];
        for (count, hit) in m.sources.iter_mut().zip(layers) {
            *count += usize::from(hit);
        }
        let wet = layers.contains(&true);
        // Water wins: a water cell is neither snow/ice nor vegetation.
        m.water.push(present(wet));
        m.snow.push(present(
            !wet && (w.snow_ice_present.contains(&classes[i]) || glaciers[i] == 1),
        ));
        m.vegetation
            .push(present(!wet && w.vegetation_present.contains(&classes[i])));
    }
    m
}

/// Generated crop-grid cover layers in a caller-owned directory.
pub(crate) struct Generated {
    pub(crate) manifest: PathBuf,
    pub(crate) note: String,
    pub(crate) observed_at: String,
}

/// Write `water.tif`, `snow-ice.tif`, `vegetation.tif` (1 absent / 2 present),
/// `land-cover.tif` (WorldCover classes) and `cover.json` for one verified crop. `existing_water` (2 = water) joins the game
/// water. Unreachable sources fail; a crop outside WorldCover coverage is absent.
pub(crate) fn generate(
    bundle: &bundle::Bundle,
    review: &Path,
    output: &Path,
    existing_water: Option<&[u8]>,
    cache: Option<&Path>,
) -> Result<Generated> {
    let (policy_bytes, policy) = load_policy(None)?;
    generate_with(
        bundle,
        review,
        output,
        existing_water,
        cache,
        &policy_bytes,
        &policy,
    )
}

fn generate_with(
    bundle: &bundle::Bundle,
    review: &Path,
    output: &Path,
    existing_water: Option<&[u8]>,
    cache: Option<&Path>,
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
    if let Some(values) = existing_water {
        ensure(
            values.len() == grid.cells(),
            "existing water mask differs from the review crop",
        )?;
    }
    let scratch = super::io_context("create cover scratch", tempfile::tempdir())?;
    let bbox = wgs84_bbox(review)?;
    let (classes, wc) = worldcover(&policy.worldcover, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("WorldCover unavailable: {e}")))?;
    let (glaciers, gl) = glims(&policy.glims, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("GLIMS unavailable: {e}")))?;
    let (mapped, os) = osm(&policy.osm, bbox, &grid, scratch.path(), cache)
        .map_err(|e| AcquisitionError::Invalid(format!("OpenStreetMap unavailable: {e}")))?;
    let w = &policy.worldcover;
    let Masks {
        water,
        snow,
        vegetation,
        sources,
    } = combine(w, &grid, &classes, &glaciers, &mapped, existing_water);
    for (name, values) in [
        ("water", &water),
        ("snow-ice", &snow),
        ("vegetation", &vegetation),
    ] {
        automatic::write_crop_mask(&output.join(format!("{name}.tif")), info, values)?;
    }
    // Raw classes, not overridden by game water; renderers pick materials from them.
    automatic::write_crop_mask(&output.join("land-cover.tif"), info, &classes)?;
    let mut land_cover = BTreeMap::new();
    for class in &classes {
        *land_cover.entry(class.to_string()).or_insert(0usize) += 1;
    }
    let count = |v: &[u8]| v.iter().filter(|x| **x == PRESENT).count();
    let mut note = format!(
        "{POLICY}: water is the tile's existing water mask, WorldCover {} class 80 (10 m steps rounded by a {} m Gaussian) and OpenStreetMap water areas and tidal flats (natural=mud, wetland=tidalflat) plus rivers, streams and canals buffered to their width ({}); snow/ice is WorldCover class 70 or inside the latest GLIMS outline per glacier ({} glaciers); vegetation is WorldCover tree, shrub, grass, crop, wetland, mangrove and moss classes; water cells are never snow/ice or vegetation; snow/ice is ground; everything else is absent. land-cover keeps the WorldCover class of every cell (0 outside coverage). Crop grid, nearest neighbour. Game judgment layers, not measurements.",
        w.observed_at,
        w.water_smooth_m,
        policy.osm.attribution,
        gl["glaciers"].as_array().map_or(0, Vec::len)
    );
    if wc["urls"].as_array().is_some_and(Vec::is_empty) {
        note.push_str(" The crop lies outside WorldCover coverage; recorded as absent (judgment).");
    }
    let manifest = json!({
        "schema_version": 2,
        "kind": "game_cover",
        "policy": POLICY,
        "policy_sha256": digest(policy_bytes),
        "policy_text": String::from_utf8_lossy(policy_bytes),
        "raw_receipt_sha256": bundle.manifest.raw_receipt_sha256,
        "review_sha256": digest(&read_small(review)?),
        "window_cells": bundle.manifest.window_cells,
        "wgs84_bbox": bbox,
        "sources": {"worldcover": wc, "glims": gl, "osm": os},
        "attribution": [policy.osm.attribution, w.citation, policy.glims.citation],
        "counts": {"cells": grid.cells(), "water_present": count(&water),
            "water_sources": {"existing": sources[0], "worldcover": sources[1], "osm": sources[2]},
            "snow_ice_present": count(&snow), "vegetation_present": count(&vegetation),
            "land_cover_classes": land_cover},
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

/// Game cover on an arbitrary metric grid: water, snow/ice and vegetation
/// (1 absent / 2 present), WorldCover classes and source details.
pub(crate) struct WorldCoverLayers {
    pub(crate) water: Vec<u8>,
    pub(crate) snow: Vec<u8>,
    pub(crate) vegetation: Vec<u8>,
    pub(crate) land_cover: Vec<u8>,
    pub(crate) details: Value,
}

pub(crate) fn world_cover(
    width: u32,
    height: u32,
    transform: [f64; 6],
    wkt: &str,
    bbox: [f64; 4],
) -> Result<WorldCoverLayers> {
    let (_, policy) = load_policy(None)?;
    let grid = Grid {
        width,
        height,
        transform,
        wkt: wkt.into(),
    };
    let scratch = super::io_context("create world cover scratch", tempfile::tempdir())?;
    let (classes, wc) = worldcover(&policy.worldcover, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("WorldCover unavailable: {e}")))?;
    let (glaciers, gl) = glims(&policy.glims, bbox, &grid, scratch.path())
        .map_err(|e| AcquisitionError::Invalid(format!("GLIMS unavailable: {e}")))?;
    let (mapped, os) = osm(&policy.osm, bbox, &grid, scratch.path(), None)
        .map_err(|e| AcquisitionError::Invalid(format!("OpenStreetMap unavailable: {e}")))?;
    let m = combine(
        &policy.worldcover,
        &grid,
        &classes,
        &glaciers,
        &mapped,
        None,
    );
    Ok(WorldCoverLayers {
        water: m.water,
        snow: m.snow,
        vegetation: m.vegetation,
        land_cover: classes,
        details: json!({"policy": POLICY, "worldcover": wc, "glims": gl, "osm": os,
            "attribution": [policy.osm.attribution, policy.worldcover.citation, policy.glims.citation]}),
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
    let generated = generate_with(
        &bundle,
        review,
        stage.path(),
        None,
        None,
        &policy_bytes,
        &policy,
    )?;
    let body: Value = super::read_json(&generated.manifest)?;
    super::io_context("publish cover draft", fs::rename(stage.path(), output))?;
    let counts = &body["counts"];
    Ok(format!(
        "{}\nWater present {}, snow/ice present {} and vegetation present {} of {} cells.\n{}",
        output.join("cover.json").display(),
        counts["water_present"],
        counts["snow_ice_present"],
        counts["vegetation_present"],
        counts["cells"],
        generated.note
    ))
}
