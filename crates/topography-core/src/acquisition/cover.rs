//! Draft public snow/ice and vegetation evidence for one reviewed native crop.
//! Drafts technical evidence only; it never writes curation or creates a vote.

use super::{
    AcquisitionError, Result, digest, ensure, hash_file, http, io_context,
    raster::{command, run},
    read_json, read_small, write_new,
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

const UNKNOWN: u8 = 0;
const ABSENT: u8 = 1;
const PRESENT: u8 = 2;
const POLICY: &str = "public_cover_draft_v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    policy: String,
    status: String,
    buffer_m: f64,
    max_evidence_bytes: u64,
    max_acquisition_span_days: i64,
    worldcover: WorldCover,
    glims: Glims,
    sentinel2: Sentinel2,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldCover {
    url_template: String,
    observed_at: String,
    classes: Vec<u8>,
    snow_ice_present: Vec<u8>,
    vegetation_present: Vec<u8>,
    vegetation_absent: Vec<u8>,
    snow_ice_band_m: f64,
    vegetation_band_m: f64,
    max_crop_cells: u64,
    license: String,
    citation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Glims {
    endpoint: String,
    type_name: String,
    line_types: Vec<String>,
    accepted_status: Vec<String>,
    reconstruction_before_year: i64,
    max_date_offset_years: f64,
    band_m: f64,
    license: String,
    citation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sentinel2 {
    stac_endpoint: String,
    collection: String,
    asset: String,
    window_days: i64,
    max_items: u32,
    max_scenes: usize,
    resolution_m: f64,
    snow: Vec<u8>,
    clear: Vec<u8>,
    unknown: Vec<u8>,
    band_m: f64,
    license: String,
    citation: String,
}

fn validate_policy(p: &Policy) -> Result<()> {
    let https = |url: &str| url.starts_with("https://") && !url.contains(['?', '#', ' ']);
    let positive = |v: f64| v.is_finite() && v > 0.0;
    let band = |v: f64| v.is_finite() && (0.0..=1000.0).contains(&v);
    let disjoint = |a: &[u8], b: &[u8]| a.iter().all(|v| !b.contains(v));
    let w = &p.worldcover;
    let s = &p.sentinel2;
    let g = &p.glims;
    ensure(
        p.schema_version == 1 && p.policy == POLICY && p.status == "draft",
        "cover policy must be schema 1, public_cover_draft_v1, status draft",
    )?;
    ensure(
        positive(p.buffer_m)
            && p.buffer_m <= 1000.0
            && (1..=1024 * 1024).contains(&p.max_evidence_bytes)
            && (0..=366).contains(&p.max_acquisition_span_days),
        "invalid cover policy buffer, evidence or date limits",
    )?;
    ensure(
        https(&w.url_template)
            && w.url_template.contains("{tile}")
            && !w.observed_at.trim().is_empty()
            && w.snow_ice_present.iter().all(|v| w.classes.contains(v))
            && w.vegetation_present.iter().all(|v| w.classes.contains(v))
            && w.vegetation_absent.iter().all(|v| w.classes.contains(v))
            && disjoint(&w.vegetation_present, &w.vegetation_absent)
            && !w.classes.contains(&0)
            && band(w.snow_ice_band_m)
            && band(w.vegetation_band_m)
            && (1..=64 * 1024 * 1024).contains(&w.max_crop_cells),
        "invalid WorldCover policy",
    )?;
    ensure(
        https(&g.endpoint)
            && !g.type_name.is_empty()
            && g.type_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
            && !g.line_types.is_empty()
            && !g.accepted_status.is_empty()
            && positive(g.max_date_offset_years)
            && band(g.band_m),
        "invalid GLIMS policy",
    )?;
    ensure(
        https(&s.stac_endpoint)
            && !s.collection.is_empty()
            && s.collection
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
            && !s.asset.is_empty()
            && (0..=90).contains(&s.window_days)
            && (1..=100).contains(&s.max_items)
            && (1..=24).contains(&s.max_scenes)
            && positive(s.resolution_m)
            && disjoint(&s.snow, &s.clear)
            && disjoint(&s.snow, &s.unknown)
            && disjoint(&s.clear, &s.unknown)
            && !s.snow.is_empty()
            && band(s.band_m),
        "invalid Sentinel-2 policy",
    )?;
    for text in [
        &w.license,
        &w.citation,
        &g.license,
        &g.citation,
        &s.license,
        &s.citation,
    ] {
        ensure(
            !text.trim().is_empty(),
            "cover policy licenses and citations must be nonempty",
        )?;
    }
    Ok(())
}

// Proleptic Gregorian day numbers (days since 1970-01-01) without extra dependencies.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, m, d) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let days = days_from_civil(y, m, d);
    // Reject impossible dates by round-tripping.
    (civil_from_days(days) == text).then_some(days)
}

fn parse_interval(text: &str) -> Option<(i64, i64)> {
    let (a, b) = text.split_once('/').unwrap_or((text, text));
    let (a, b) = (parse_date(a)?, parse_date(b)?);
    (a <= b).then_some((a, b))
}

fn interval_text((a, b): (i64, i64)) -> String {
    if a == b {
        civil_from_days(a)
    } else {
        format!("{}/{}", civil_from_days(a), civil_from_days(b))
    }
}

fn distance((a, b): (i64, i64), day: i64) -> i64 {
    if day < a {
        a - day
    } else if day > b {
        day - b
    } else {
        0
    }
}

struct Acquisition {
    interval: Option<(i64, i64)>,
    origin: &'static str,
    note: String,
}

fn acquisition(source_dates: &[String], flag: Option<&str>) -> Result<Acquisition> {
    if let Some(flag) = flag {
        let interval = parse_interval(flag).ok_or_else(|| {
            AcquisitionError::Invalid("--acquired must be YYYY-MM-DD[/YYYY-MM-DD]".into())
        })?;
        return Ok(Acquisition {
            interval: Some(interval),
            origin: "command_flag",
            note: format!("supplied by --acquired {flag}"),
        });
    }
    let mut interval: Option<(i64, i64)> = None;
    for entry in source_dates {
        let token = entry.split_whitespace().next().unwrap_or("");
        let Some((a, b)) = parse_interval(token) else {
            return Ok(Acquisition {
                interval: None,
                origin: "none",
                note: format!("source acquisition date is not machine-readable: {entry:?}"),
            });
        };
        interval = Some(interval.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
    }
    Ok(match interval {
        Some(interval) => Acquisition {
            interval: Some(interval),
            origin: "source_metadata",
            note: "parsed from source acquisition_dates".into(),
        },
        None => Acquisition {
            interval: None,
            origin: "none",
            note: "source records no acquisition date".into(),
        },
    })
}

struct Grid {
    width: u32,
    height: u32,
    transform: [f64; 6],
    wkt: String,
    mapping: String,
}

impl Grid {
    /// Georeferenced extent of a cell window as [xmin, ymin, xmax, ymax].
    fn extent(&self, [x, y, w, h]: [u32; 4]) -> [f64; 4] {
        let t = &self.transform;
        let xmin = t[0] + f64::from(x) * t[1];
        let ymax = t[3] + f64::from(y) * t[5];
        [
            xmin,
            ymax + f64::from(h) * t[5],
            xmin + f64::from(w) * t[1],
            ymax,
        ]
    }

    fn cells(&self, metres: f64, axis: usize) -> u32 {
        let spacing = self.transform[if axis == 0 { 1 } else { 5 }].abs();
        (metres / spacing - 1e-9).ceil().max(0.0) as u32
    }

    fn srs_xml(&self) -> String {
        format!(
            "<SRS dataAxisToSRSAxisMapping=\"{}\">{}</SRS><GeoTransform>{}</GeoTransform>",
            self.mapping,
            xml_escape(&self.wkt),
            self.transform.map(|v| v.to_string()).join(",")
        )
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn source_grid(raw: &Path, receipt: &super::RawReceipt) -> Result<Grid> {
    let objects = raw
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AcquisitionError::Invalid("raw receipt needs storage root".into()))?
        .join("objects");
    let info: Value = read_json(&objects.join(&receipt.inspection_sha256))?;
    let transform: [f64; 6] = serde_json::from_value(info["geoTransform"].clone())?;
    let [width, height]: [u32; 2] = serde_json::from_value(info["size"].clone())?;
    ensure(
        transform[1] > 0.0 && transform[5] < 0.0 && transform[2] == 0.0 && transform[4] == 0.0,
        "cover drafting requires a north-up source grid",
    )?;
    let mapping: Vec<u64> =
        serde_json::from_value(info["coordinateSystem"]["dataAxisToSRSAxisMapping"].clone())?;
    Ok(Grid {
        width,
        height,
        transform,
        wkt: info["coordinateSystem"]["wkt"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AcquisitionError::Invalid("source CRS WKT missing".into()))?
            .into(),
        mapping: mapping
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    })
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

/// Target a cell window of the source grid with nearest-neighbour categories.
fn warp_to_window(input: &str, output: &Path, grid: &Grid, window: [u32; 4]) -> Result<()> {
    let e = grid.extent(window).map(|v| v.to_string());
    run(gdal("gdalwarp")
        .args(["-q", "-r", "near", "-et", "0", "-ot", "Byte"])
        .args(["-wo", "INIT_DEST=0", "-t_srs", &grid.wkt, "-te"])
        .args(&e)
        .args(["-ts", &window[2].to_string(), &window[3].to_string()])
        .args(["-of", "GTiff", input])
        .arg(output))?;
    Ok(())
}

fn read_window(path: &Path, scratch: &Path, name: &str, window: [u32; 4]) -> Result<Vec<u8>> {
    review::band(
        path,
        scratch,
        name,
        window[2] as usize * window[3] as usize,
        false,
    )
}

fn wgs84_bbox(grid: &Grid, window: [u32; 4], scratch: &Path) -> Result<[f64; 4]> {
    let template = scratch.join("work-template.vrt");
    let mut local = Grid {
        transform: grid.transform,
        wkt: grid.wkt.clone(),
        mapping: grid.mapping.clone(),
        width: window[2],
        height: window[3],
    };
    let [xmin, _, _, ymax] = grid.extent(window);
    local.transform[0] = xmin;
    local.transform[3] = ymax;
    write_new(
        &template,
        format!(
            "<VRTDataset rasterXSize=\"{}\" rasterYSize=\"{}\">{}<VRTRasterBand dataType=\"Byte\" band=\"1\"/></VRTDataset>",
            local.width,
            local.height,
            local.srs_xml()
        )
        .as_bytes(),
    )?;
    let info: Value = serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-nomd"])
        .arg(&template))?)?;
    let ring = info["wgs84Extent"]["coordinates"][0]
        .as_array()
        .filter(|r| r.len() >= 4)
        .ok_or_else(|| AcquisitionError::Invalid("work window has no WGS84 extent".into()))?;
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
        "work window WGS84 extent exceeds one degree or crosses the antimeridian",
    )?;
    Ok(bbox)
}

fn coord(v: f64) -> String {
    format!("{v:.7}")
}

struct Layer {
    values: Vec<u8>,
    status: String,
    details: Value,
    evidence: Vec<Value>,
}

impl Layer {
    fn unknown(cells: usize, status: impl Into<String>, details: Value) -> Self {
        Self {
            values: vec![UNKNOWN; cells],
            status: status.into(),
            details,
            evidence: Vec::new(),
        }
    }
}

struct Context<'a> {
    policy: &'a Policy,
    grid: &'a Grid,
    work: [u32; 4],
    bbox: [f64; 4],
    stage: &'a Path,
    scratch: &'a Path,
}

impl Context<'_> {
    fn evidence(
        &self,
        id: &str,
        file: &str,
        description: String,
        url: Option<&str>,
    ) -> Result<Value> {
        let (sha256, bytes) = hash_file(&self.stage.join(file))?;
        ensure(
            bytes <= self.policy.max_evidence_bytes,
            format!(
                "{file} evidence exceeds {} bytes",
                self.policy.max_evidence_bytes
            ),
        )?;
        Ok(json!({"id":id,"path":file,"sha256":sha256,"description":description,"source_url":url}))
    }
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

/// Crop the public WorldCover COG(s) and place categories on the work window.
fn worldcover(ctx: &Context) -> Result<(Vec<u8>, Value, Value)> {
    let w = &ctx.policy.worldcover;
    let mut available = Vec::new();
    let mut missing = Vec::new();
    for tile in worldcover_tiles(ctx.bbox) {
        let url = w.url_template.replace("{tile}", &tile);
        match run(gdal("gdalinfo")
            .args(["-json", "-nomd", "-noct"])
            .arg(format!("/vsicurl/{url}")))
        {
            Ok(_) => available.push(url),
            Err(error) => missing.push(json!({"tile":tile,"url":url,"error":error.to_string()})),
        }
    }
    ensure(
        !available.is_empty(),
        format!("no WorldCover tile could be opened: {}", json!(missing)),
    )?;
    let vrt = ctx.scratch.join("worldcover.vrt");
    run(gdal("gdalbuildvrt")
        .arg("-q")
        .arg(&vrt)
        .args(available.iter().map(|u| format!("/vsicurl/{u}"))))?;
    // About 100 m of padding so corner transforms cannot clip the window.
    let [west, south, east, north] = ctx.bbox;
    let (west, east, south, north) = (west - 1e-3, east + 1e-3, south - 1e-3, north + 1e-3);
    // Plan the crop as a VRT first so the cell limit is checked before pixel reads.
    let planned = ctx.scratch.join("worldcover-crop.vrt");
    run(gdal("gdal_translate")
        .args(["-q", "-of", "VRT", "-projwin_srs", "EPSG:4326", "-projwin"])
        .args([west, north, east, south].map(coord))
        .arg(&vrt)
        .arg(&planned))?;
    let info: Value = serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-nomd", "-noct"])
        .arg(&planned))?)?;
    let [cw, ch]: [u64; 2] = serde_json::from_value(info["size"].clone())?;
    ensure(
        cw * ch <= w.max_crop_cells,
        "WorldCover crop exceeds the policy cell limit",
    )?;
    let file = "cover-worldcover.tif";
    run(gdal("gdal_translate")
        .args(["-q", "-of", "GTiff", "-co", "COMPRESS=DEFLATE"])
        .arg(&planned)
        .arg(ctx.stage.join(file)))?;
    let evidence = ctx.evidence(
        "cover-worldcover",
        file,
        format!(
            "ESA WorldCover v200 2021 10 m map crop (unmodified native pixels) from {}. License: {}. Citation: {}",
            available.join(", "),
            w.license,
            w.citation
        ),
        Some(&available[0]),
    )?;
    let placed = ctx.scratch.join("worldcover-work.tif");
    warp_to_window(
        &ctx.stage.join(file).to_string_lossy(),
        &placed,
        ctx.grid,
        ctx.work,
    )?;
    let values = read_window(&placed, ctx.scratch, "worldcover-work", ctx.work)?;
    Ok((
        values,
        evidence,
        json!({"tiles":available,"missing_tiles":missing}),
    ))
}

fn glims_date(feature: &Value) -> Option<i64> {
    parse_date(feature["properties"]["src_date"].as_str()?.get(..10)?)
}

/// Query GLIMS outlines, choose one outline per glacier and rasterize it.
fn glims(ctx: &Context, interval: Option<(i64, i64)>) -> Result<Layer> {
    let g = &ctx.policy.glims;
    let [west, south, east, north] = ctx.bbox;
    let url = format!(
        "{}?service=WFS&version=2.0.0&request=GetFeature&typeNames={}&outputFormat=application/json&srsName=EPSG:4326&bbox={},{},{},{},urn:ogc:def:crs:EPSG::4326",
        g.endpoint,
        g.type_name,
        coord(south),
        coord(west),
        coord(north),
        coord(east)
    );
    let bytes = http::query(&url)?;
    let file = "cover-glims.json";
    write_new(&ctx.stage.join(file), &bytes)?;
    let body: Value = serde_json::from_slice(&bytes)?;
    let features = body["features"]
        .as_array()
        .filter(|_| body["type"] == "FeatureCollection")
        .ok_or_else(|| {
            AcquisitionError::Invalid("GLIMS response is not a FeatureCollection".into())
        })?;
    if let Some(matched) = body["numberMatched"].as_u64() {
        ensure(
            matched as usize == features.len(),
            "GLIMS response is truncated; narrow the window",
        )?;
    }
    let evidence = ctx.evidence(
        "cover-glims",
        file,
        format!(
            "GLIMS glacier outline WFS response for the work window bbox. License: {}. Citation: {}",
            g.license, g.citation
        ),
        Some(&url),
    )?;
    let mut glaciers: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let (mut other_lines, mut reconstructions) = (0, 0);
    for feature in features {
        let p = &feature["properties"];
        let line = p["line_type"].as_str().unwrap_or("");
        if !g.line_types.iter().any(|t| t == line) {
            other_lines += 1;
            continue;
        }
        let id = p["glac_id"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AcquisitionError::Invalid("GLIMS outline lacks glac_id".into()))?;
        let date = glims_date(feature)
            .ok_or_else(|| AcquisitionError::Invalid("GLIMS outline lacks src_date".into()))?;
        ensure(
            matches!(
                feature["geometry"]["type"].as_str(),
                Some("Polygon" | "MultiPolygon")
            ),
            "GLIMS outline geometry must be a polygon",
        )?;
        // GeoServer declares a lat/lon URN but writes lon/lat; verify rather than assume.
        let first = &feature["geometry"]["coordinates"];
        let mut point = first;
        while point[0].is_array() {
            point = &point[0];
        }
        let (lon, lat) = (point[0].as_f64(), point[1].as_f64());
        ensure(
            lon.zip(lat).is_some_and(|(lon, lat)| {
                (west - 1.0..=east + 1.0).contains(&lon)
                    && (south - 1.0..=north + 1.0).contains(&lat)
            }),
            "GLIMS coordinates are not lon/lat near the requested bbox",
        )?;
        if civil_from_days(date)[..4].parse::<i64>().unwrap_or(0) < g.reconstruction_before_year {
            reconstructions += 1;
            continue;
        }
        glaciers.entry(id.into()).or_default().push(feature);
    }
    let limit = (g.max_date_offset_years * 365.25).round() as i64;
    let mut burns = Vec::new();
    let mut chosen = Vec::new();
    for (id, mut outlines) in glaciers {
        outlines.sort_by_key(|f| {
            let date = glims_date(f).unwrap_or(0);
            (
                interval.map_or(i64::MAX, |i| distance(i, date)),
                -date,
                f["properties"]["anlys_id"].as_i64().unwrap_or(0),
            )
        });
        let feature = outlines[0];
        let date = glims_date(feature).unwrap_or(0);
        let offset = interval.map(|i| distance(i, date));
        let status_ok = feature["properties"]["rec_status"]
            .as_str()
            .is_some_and(|s| g.accepted_status.iter().any(|a| a == s));
        let current = status_ok && offset.is_some_and(|d| d <= limit);
        burns.push((current, feature["geometry"].clone()));
        chosen.push(json!({"glac_id":id,"anlys_id":feature["properties"]["anlys_id"],"src_date":civil_from_days(date),
            "offset_days":offset,"rec_status":feature["properties"]["rec_status"],
            "use": if current {"present"} else {"unknown (date offset, missing date or status)"}}));
    }
    // Stale outlines first so current outlines win where both overlap.
    burns.sort_by_key(|(current, _)| *current);
    let raster = ctx.scratch.join("glims.tif");
    let e = ctx.grid.extent(ctx.work);
    run(command("gdal_create")
        .args([
            "-q", "-of", "GTiff", "-bands", "1", "-ot", "Byte", "-burn", "1",
        ])
        .args([
            "-outsize",
            &ctx.work[2].to_string(),
            &ctx.work[3].to_string(),
        ])
        .args(["-a_srs", &ctx.grid.wkt, "-a_ullr"])
        .args([e[0], e[3], e[2], e[1]].map(|v| v.to_string()))
        .arg(&raster))?;
    if !burns.is_empty() {
        let vector = ctx.scratch.join("glims-burn.geojson");
        let features: Vec<Value> = burns
            .into_iter()
            .map(|(current, geometry)| json!({"type":"Feature","properties":{"burn": if current {2} else {3}},"geometry":geometry}))
            .collect();
        write_new(
            &vector,
            &serde_json::to_vec(&json!({"type":"FeatureCollection","features":features}))?,
        )?;
        run(command("gdal_rasterize")
            .args(["-q", "-a", "burn"])
            .arg(&vector)
            .arg(&raster))?;
    }
    let burned = read_window(&raster, ctx.scratch, "glims-work", ctx.work)?;
    let (w, h) = (ctx.work[2] as usize, ctx.work[3] as usize);
    let mut values: Vec<u8> = burned
        .iter()
        .map(|v| if *v >= 2 { PRESENT } else { ABSENT })
        .collect();
    let (rx, ry) = (ctx.grid.cells(g.band_m, 0), ctx.grid.cells(g.band_m, 1));
    boundary_band(&mut values, w, h, rx, ry);
    for (value, burn) in values.iter_mut().zip(&burned) {
        if *burn == 3 {
            *value = UNKNOWN;
        }
    }
    Ok(Layer {
        values,
        status: "used".into(),
        details: json!({"glaciers":chosen,"reconstructions_excluded":reconstructions,"other_line_types_ignored":other_lines}),
        evidence: vec![evidence],
    })
}

/// Sentinel-2 L2A scene classification near the acquisition interval.
fn sentinel2(ctx: &Context, interval: (i64, i64)) -> Result<Layer> {
    let s = &ctx.policy.sentinel2;
    let window = (interval.0 - s.window_days, interval.1 + s.window_days);
    let [west, south, east, north] = ctx.bbox;
    let url = format!(
        "{}/search?collections={}&bbox={},{},{},{}&datetime={}T00:00:00Z/{}T23:59:59Z&limit={}",
        s.stac_endpoint.trim_end_matches('/'),
        s.collection,
        coord(west),
        coord(south),
        coord(east),
        coord(north),
        civil_from_days(window.0),
        civil_from_days(window.1),
        s.max_items
    );
    let bytes = http::query(&url)?;
    let file = "cover-sentinel2-stac.json";
    write_new(&ctx.stage.join(file), &bytes)?;
    let body: Value = serde_json::from_slice(&bytes)?;
    let features = body["features"]
        .as_array()
        .ok_or_else(|| AcquisitionError::Invalid("STAC response lacks features".into()))?;
    let matched = body["numberMatched"]
        .as_u64()
        .or(body["context"]["matched"].as_u64());
    ensure(
        matched.is_none_or(|m| m as usize <= features.len()),
        "STAC search is truncated; narrow the window",
    )?;
    let mut evidence = vec![ctx.evidence(
        "cover-sentinel2-stac",
        file,
        format!(
            "Earth Search STAC {} search response for {}..{}. License: {}. Citation: {}",
            s.collection,
            civil_from_days(window.0),
            civil_from_days(window.1),
            s.license,
            s.citation
        ),
        Some(&url),
    )?];
    let mut scenes = Vec::new();
    for item in features {
        let date = item["properties"]["datetime"]
            .as_str()
            .and_then(|d| parse_date(d.get(..10)?))
            .ok_or_else(|| AcquisitionError::Invalid("STAC item lacks datetime".into()))?;
        ensure(
            (window.0..=window.1).contains(&date),
            "STAC item lies outside the requested window",
        )?;
        let href = item["assets"][s.asset.as_str()]["href"]
            .as_str()
            .filter(|h| h.starts_with("https://") && h.bytes().all(|b| b.is_ascii_graphic()))
            .ok_or_else(|| {
                AcquisitionError::Invalid(format!("STAC item lacks an HTTPS {} asset", s.asset))
            })?;
        let id = item["id"].as_str().unwrap_or("").to_string();
        scenes.push((distance(interval, date), date, id, href.to_string()));
    }
    scenes.sort();
    let found = scenes.len();
    scenes.truncate(s.max_scenes);
    let cells = ctx.work[2] as usize * ctx.work[3] as usize;
    let mut used = Vec::new();
    let mut failed = Vec::new();
    let mut snow = vec![false; cells];
    let mut clear = vec![false; cells];
    let e = ctx.grid.extent(ctx.work).map(|v| v.to_string());
    let res = s.resolution_m.to_string();
    for (_, date, id, href) in &scenes {
        let crop = ctx.scratch.join(format!("scl-{}.tif", used.len() + 1));
        let fetched = run(gdal("gdalwarp")
            .args(["-q", "-r", "near", "-et", "0", "-ot", "Byte", "-tap"])
            .args(["-wo", "INIT_DEST=0", "-t_srs", &ctx.grid.wkt, "-te"])
            .args(&e)
            .args(["-tr", &res, &res, "-of", "GTiff", "-co", "COMPRESS=DEFLATE"])
            .arg(format!("/vsicurl/{href}"))
            .arg(&crop));
        if let Err(error) = fetched {
            failed.push(json!({"id":id,"href":href,"error":error.to_string()}));
            continue;
        }
        let placed = ctx.scratch.join(format!("scl-work-{}.tif", used.len() + 1));
        warp_to_window(&crop.to_string_lossy(), &placed, ctx.grid, ctx.work)?;
        let values = read_window(
            &placed,
            ctx.scratch,
            &format!("scl-work-{}", used.len() + 1),
            ctx.work,
        )?;
        for (i, v) in values.iter().enumerate() {
            snow[i] |= s.snow.contains(v);
            clear[i] |= s.clear.contains(v);
        }
        used.push(json!({"band":used.len() + 1,"id":id,"date":civil_from_days(*date),"href":href}));
    }
    if !used.is_empty() {
        let vrt = ctx.scratch.join("scl.vrt");
        run(command("gdalbuildvrt")
            .args(["-q", "-separate"])
            .arg(&vrt)
            .args((1..=used.len()).map(|i| ctx.scratch.join(format!("scl-{i}.tif")))))?;
        let file = "cover-sentinel2-scl.tif";
        run(command("gdal_translate")
            .args(["-q", "-of", "GTiff", "-co", "COMPRESS=DEFLATE"])
            .arg(&vrt)
            .arg(ctx.stage.join(file)))?;
        evidence.push(ctx.evidence(
            "cover-sentinel2-scl",
            file,
            format!(
                "Sentinel-2 L2A SCL crops, one band per scene in cover.json order, nearest-neighbour on the DEM CRS at {} m. License: {}. Citation: {}",
                s.resolution_m, s.license, s.citation
            ),
            used[0]["href"].as_str(),
        )?);
    }
    let (w, h) = (ctx.work[2] as usize, ctx.work[3] as usize);
    let mut values: Vec<u8> = snow
        .iter()
        .zip(&clear)
        .map(|(snow, clear)| match (snow, clear) {
            (true, _) => PRESENT,
            (false, true) => ABSENT,
            _ => UNKNOWN,
        })
        .collect();
    boundary_band(
        &mut values,
        w,
        h,
        ctx.grid.cells(s.band_m, 0),
        ctx.grid.cells(s.band_m, 1),
    );
    let status = if used.is_empty() {
        if found == 0 { "no_scenes" } else { "failed" }
    } else {
        "used"
    };
    Ok(Layer {
        values,
        status: status.into(),
        details: json!({"window":interval_text(window),"scenes_found":found,"scenes":used,"failed_scenes":failed}),
        evidence,
    })
}

/// Mark cells within a square band around present/absent class changes as unknown.
fn boundary_band(values: &mut [u8], w: usize, h: usize, rx: u32, ry: u32) {
    if rx == 0 && ry == 0 {
        return;
    }
    let mut edge = vec![false; values.len()];
    let differs = |a: u8, b: u8| (a == PRESENT && b == ABSENT) || (a == ABSENT && b == PRESENT);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x + 1 < w && differs(values[i], values[i + 1]) {
                edge[i] = true;
                edge[i + 1] = true;
            }
            if y + 1 < h && differs(values[i], values[i + w]) {
                edge[i] = true;
                edge[i + w] = true;
            }
        }
    }
    let dilate = |input: &[bool],
                  n: usize,
                  len: usize,
                  stride: usize,
                  lines: usize,
                  step: usize,
                  r: usize| {
        let mut out = vec![false; n];
        let mut prefix = vec![0usize; len + 1];
        for line in 0..lines {
            for k in 0..len {
                prefix[k + 1] = prefix[k] + usize::from(input[line * step + k * stride]);
            }
            for k in 0..len {
                let (a, b) = (k.saturating_sub(r), (k + r + 1).min(len));
                out[line * step + k * stride] = prefix[b] > prefix[a];
            }
        }
        out
    };
    let horizontal = dilate(&edge, values.len(), w, 1, h, w, rx as usize);
    let band = dilate(&horizontal, values.len(), h, w, w, 1, ry as usize);
    for (value, banded) in values.iter_mut().zip(band) {
        if banded {
            *value = UNKNOWN;
        }
    }
}

/// Write a complete source-grid Byte mask; cells outside the work window are unknown.
fn write_mask(ctx: &Context, values: &[u8], name: &str) -> Result<PathBuf> {
    let [x, y, w, h] = ctx.work;
    let bin = ctx.scratch.join(format!("{name}.bin"));
    write_new(&bin, values)?;
    let work = ctx.scratch.join(format!("{name}-work.vrt"));
    write_new(
        &work,
        format!(
            "<VRTDataset rasterXSize=\"{w}\" rasterYSize=\"{h}\"><VRTRasterBand dataType=\"Byte\" band=\"1\" subClass=\"VRTRawRasterBand\"><SourceFilename relativeToVRT=\"1\">{name}.bin</SourceFilename><ImageOffset>0</ImageOffset><PixelOffset>1</PixelOffset><LineOffset>{w}</LineOffset><ByteOrder>LSB</ByteOrder></VRTRasterBand></VRTDataset>"
        )
        .as_bytes(),
    )?;
    let full = ctx.scratch.join(format!("{name}-full.vrt"));
    write_new(
        &full,
        format!(
            "<VRTDataset rasterXSize=\"{}\" rasterYSize=\"{}\">{}<VRTRasterBand dataType=\"Byte\" band=\"1\"><SimpleSource><SourceFilename relativeToVRT=\"1\">{name}-work.vrt</SourceFilename><SourceBand>1</SourceBand><SrcRect xOff=\"0\" yOff=\"0\" xSize=\"{w}\" ySize=\"{h}\"/><DstRect xOff=\"{x}\" yOff=\"{y}\" xSize=\"{w}\" ySize=\"{h}\"/></SimpleSource></VRTRasterBand></VRTDataset>",
            ctx.grid.width,
            ctx.grid.height,
            ctx.grid.srs_xml()
        )
        .as_bytes(),
    )?;
    let output = ctx.stage.join(format!("{name}.tif"));
    run(command("gdal_translate")
        .args(["-q", "-of", "GTiff", "-mask", "none", "-a_nodata", "none"])
        .args(["-co", "COMPRESS=DEFLATE", "-co", "TILED=YES"])
        .arg(&full)
        .arg(&output))?;
    Ok(output)
}

#[derive(Default)]
struct Counts {
    unknown: u64,
    absent: u64,
    present: u64,
}

fn counts(values: &[u8], work: [u32; 4], review: [u32; 4]) -> Counts {
    let mut c = Counts::default();
    let w = work[2] as usize;
    for row in review[1]..review[1] + review[3] {
        let start = (row - work[1]) as usize * w + (review[0] - work[0]) as usize;
        for v in &values[start..start + review[2] as usize] {
            match *v {
                PRESENT => c.present += 1,
                ABSENT => c.absent += 1,
                _ => c.unknown += 1,
            }
        }
    }
    c
}

fn counts_json(c: &Counts) -> Value {
    json!({"unknown":c.unknown,"absent":c.absent,"present":c.present})
}

/// Fetch public cover evidence for one review crop and write a new cover draft directory.
pub fn draft(
    raw: &Path,
    review: &Path,
    policy_path: &Path,
    output: &Path,
    acquired: Option<&str>,
) -> Result<String> {
    ensure(
        !output.exists(),
        "cover output already exists; choose a new directory",
    )?;
    let policy_bytes = read_small(policy_path)?;
    let policy: Policy = toml::from_str(
        std::str::from_utf8(&policy_bytes)
            .map_err(|_| AcquisitionError::Invalid("cover policy must be UTF-8".into()))?,
    )
    .map_err(|e| AcquisitionError::Invalid(format!("invalid cover policy: {e}")))?;
    validate_policy(&policy)?;
    let receipt = super::verify(raw)?;
    let bundle = bundle::verify_with_raw(raw, review, &receipt)?;
    let grid = source_grid(raw, &receipt)?;
    let date = acquisition(&bundle.source.acquisition_dates, acquired)?;
    let review_window = bundle.manifest.window_cells;
    let [x, y, w, h] = review_window;
    let (bx, by) = (
        grid.cells(policy.buffer_m, 0),
        grid.cells(policy.buffer_m, 1),
    );
    let (x0, y0) = (x.saturating_sub(bx), y.saturating_sub(by));
    let (x1, y1) = (
        (x + w).saturating_add(bx).min(grid.width),
        (y + h).saturating_add(by).min(grid.height),
    );
    let work = [x0, y0, x1 - x0, y1 - y0];
    ensure(
        u64::from(work[2]) * u64::from(work[3]) <= 16 * 1024 * 1024,
        "cover work window exceeds 16 Mi cells",
    )?;
    let cells = work[2] as usize * work[3] as usize;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create cover parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage cover draft", tempfile::tempdir_in(parent))?;
    let scratch = io_context("create cover scratch", tempfile::tempdir())?;
    let bbox = wgs84_bbox(&grid, work, scratch.path())?;
    let ctx = Context {
        policy: &policy,
        grid: &grid,
        work,
        bbox,
        stage: stage.path(),
        scratch: scratch.path(),
    };
    write_new(&stage.path().join("cover-policy.toml"), &policy_bytes)?;
    let policy_evidence = ctx.evidence(
        "cover-policy",
        "cover-policy.toml",
        "Versioned draft cover-source policy used for this automated draft".into(),
        None,
    )?;
    let mut evidence = vec![policy_evidence];

    let mut limitations = Vec::new();
    let (wc_snow, wc_veg, wc_status) = match worldcover(&ctx) {
        Ok((values, item, details)) => {
            evidence.push(item);
            let wc = &policy.worldcover;
            let map = |v: &u8, present: &[u8], absent: &dyn Fn(&u8) -> bool| {
                if present.contains(v) {
                    PRESENT
                } else if absent(v) {
                    ABSENT
                } else {
                    UNKNOWN
                }
            };
            let snow_absent = |v: &u8| wc.classes.contains(v) && !wc.snow_ice_present.contains(v);
            let veg_absent = |v: &u8| wc.vegetation_absent.contains(v);
            let mut snow: Vec<u8> = values
                .iter()
                .map(|v| map(v, &wc.snow_ice_present, &snow_absent))
                .collect();
            let mut veg: Vec<u8> = values
                .iter()
                .map(|v| map(v, &wc.vegetation_present, &veg_absent))
                .collect();
            let (ww, wh) = (work[2] as usize, work[3] as usize);
            boundary_band(
                &mut snow,
                ww,
                wh,
                grid.cells(wc.snow_ice_band_m, 0),
                grid.cells(wc.snow_ice_band_m, 1),
            );
            boundary_band(
                &mut veg,
                ww,
                wh,
                grid.cells(wc.vegetation_band_m, 0),
                grid.cells(wc.vegetation_band_m, 1),
            );
            (
                Some(snow),
                Some(veg),
                json!({"status":"used","details":details}),
            )
        }
        Err(error) => {
            limitations.push(format!("WorldCover unavailable: {error}"));
            (
                None,
                None,
                json!({"status":"failed","error":error.to_string()}),
            )
        }
    };
    let glims_layer = glims(&ctx, date.interval).unwrap_or_else(|error| {
        limitations.push(format!("GLIMS unavailable: {error}"));
        Layer::unknown(cells, "failed", json!({"error":error.to_string()}))
    });
    let seasonal = match date.interval {
        None => {
            limitations.push(format!(
                "No usable DEM acquisition date ({}); seasonal snow cannot be assessed, so snow/ice cannot be absent.",
                date.note
            ));
            Layer::unknown(cells, "skipped", json!({"reason":date.note}))
        }
        Some((a, b)) if b - a > policy.max_acquisition_span_days => {
            let reason = format!(
                "DEM acquisition interval {} spans {} days, more than the policy {} days; seasonal snow cannot be assessed.",
                interval_text((a, b)),
                b - a,
                policy.max_acquisition_span_days
            );
            limitations.push(reason.clone());
            Layer::unknown(cells, "skipped", json!({"reason":reason}))
        }
        Some(interval) => sentinel2(&ctx, interval).unwrap_or_else(|error| {
            Layer::unknown(cells, "failed", json!({"error":error.to_string()}))
        }),
    };
    if seasonal.status == "failed" {
        limitations.push(format!(
            "Sentinel-2 seasonal check failed: {}",
            seasonal.details
        ));
    } else if seasonal.status == "no_scenes" {
        limitations.push("No Sentinel-2 L2A scene lies within the seasonal window.".into());
    }
    evidence.extend(glims_layer.evidence.iter().cloned());
    evidence.extend(seasonal.evidence.iter().cloned());

    let wc_snow_values = wc_snow.unwrap_or_else(|| vec![UNKNOWN; cells]);
    let snow: Vec<u8> = (0..cells)
        .map(|i| {
            let layers = [wc_snow_values[i], glims_layer.values[i], seasonal.values[i]];
            if layers.contains(&PRESENT) {
                PRESENT
            } else if layers.iter().all(|v| *v == ABSENT) {
                ABSENT
            } else {
                UNKNOWN
            }
        })
        .collect();
    let snow_counts = counts(&snow, work, review_window);
    let snow_path = write_mask(&ctx, &snow, "snow-ice")?;
    let veg_values = wc_veg.clone().unwrap_or_else(|| vec![UNKNOWN; cells]);
    let veg_counts = counts(&veg_values, work, review_window);
    let veg_path = write_mask(&ctx, &veg_values, "vegetation")?;
    for (path, name) in [(&snow_path, "snow-ice"), (&veg_path, "vegetation")] {
        automatic::verify_source_cover(path, raw, &receipt, name)?;
    }

    let ids = |items: &[Value], allowed: &[&str]| -> Vec<Value> {
        items
            .iter()
            .filter(|e| allowed.contains(&e["id"].as_str().unwrap_or("")))
            .map(|e| e["id"].clone())
            .collect()
    };
    let observed = date.interval.map(interval_text);
    let summary = |c: &Counts| {
        format!(
            "review window {} present, {} absent, {} unknown cells",
            c.present, c.absent, c.unknown
        )
    };
    let snow_note = format!(
        "Automated {POLICY} draft: ESA WorldCover 2021 class 70 ({}), GLIMS outlines nearest the DEM date ({}), Sentinel-2 SCL seasonal check ({}); {}. Coarse sources placed by nearest neighbour with boundary bands marked unknown.",
        wc_status["status"].as_str().unwrap_or("failed"),
        glims_layer.status,
        seasonal.status,
        summary(&snow_counts)
    );
    let snow_ids = ids(
        &evidence,
        &[
            "cover-policy",
            "cover-worldcover",
            "cover-glims",
            "cover-sentinel2-stac",
            "cover-sentinel2-scl",
        ],
    );
    let full_window = json!([review_window]);
    let snow_descriptor = if snow_counts.unknown == 0 && snow_counts.present == 0 {
        json!({"kind":"all_absent","windows":full_window,"note":snow_note,"observed_at":observed,"evidence_ids":snow_ids})
    } else {
        json!({"kind":"mask","path":"snow-ice.tif","sha256":hash_file(&snow_path)?.0,"note":snow_note,"observed_at":observed,"evidence_ids":snow_ids})
    };
    let veg_ids = ids(&evidence, &["cover-policy", "cover-worldcover"]);
    let veg_note = format!(
        "Automated {POLICY} draft from ESA WorldCover 2021 10 m classes placed by nearest neighbour; epoch 2021 may differ from the DEM date; {}.",
        summary(&veg_counts)
    );
    let wc_observed = &policy.worldcover.observed_at;
    let veg_descriptor = if wc_veg.is_none() {
        json!({"kind":"unavailable","note":format!("No usable public vegetation classification: {}", wc_status["error"].as_str().unwrap_or("source failed")),"observed_at":null,"evidence_ids":veg_ids})
    } else if veg_counts.unknown == 0 && veg_counts.present == 0 {
        json!({"kind":"all_absent","windows":full_window,"note":veg_note,"observed_at":wc_observed,"evidence_ids":veg_ids})
    } else {
        json!({"kind":"mask","path":"vegetation.tif","sha256":hash_file(&veg_path)?.0,"note":veg_note,"observed_at":wc_observed,"evidence_ids":veg_ids})
    };
    let resolved = snow_counts.unknown == 0;
    if !resolved {
        limitations.push(format!(
            "Snow/ice is unresolved for {} review cells; automatic preparation will block until the extent is resolved (technical attention needed).",
            snow_counts.unknown
        ));
    }
    if veg_counts.unknown > 0 && wc_veg.is_some() {
        limitations.push(format!(
            "Vegetation unknown for {} review cells (optional layer; does not block ground preparation).",
            veg_counts.unknown
        ));
    }
    let manifest = json!({
        "schema_version": 1,
        "kind": "cover_draft",
        "policy": POLICY,
        "policy_sha256": digest(&policy_bytes),
        "raw_receipt": {"path": fs::canonicalize(raw).map_err(|e| AcquisitionError::Io{context:"resolve raw receipt".into(),source:e})?, "sha256": digest(&read_small(raw)?)},
        "review": {"path": fs::canonicalize(review).map_err(|e| AcquisitionError::Io{context:"resolve review".into(),source:e})?, "sha256": digest(&read_small(review)?)},
        "source_sha256": receipt.source_sha256,
        "asset_sha256": receipt.asset_sha256,
        "grid": {"size":[grid.width,grid.height],"geoTransform":grid.transform},
        "review_window": review_window,
        "work_window": work,
        "wgs84_bbox": bbox,
        "acquisition": {"interval": observed, "origin": date.origin, "note": date.note, "source_dates": bundle.source.acquisition_dates},
        "sources": {
            "worldcover": wc_status,
            "glims": {"status": glims_layer.status, "details": glims_layer.details},
            "sentinel2": {"status": seasonal.status, "details": seasonal.details},
        },
        "review_counts": {"snow_ice": counts_json(&snow_counts), "vegetation": counts_json(&veg_counts),
            "worldcover_snow_ice": counts_json(&counts(&wc_snow_values, work, review_window)),
            "glims": counts_json(&counts(&glims_layer.values, work, review_window)),
            "seasonal": counts_json(&counts(&seasonal.values, work, review_window))},
        "evidence": evidence,
        "snow_ice": snow_descriptor,
        "vegetation": veg_descriptor,
        "snow_ice_resolved": resolved,
        "limitations": limitations,
        "admission": "technical draft only; no curation, vote or acceptance",
    });
    write_new(
        &stage.path().join("cover.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    let line = |name: &str, c: &Counts, kind: &Value| {
        format!(
            "{name}: {} (present {}, absent {}, unknown {})",
            kind.as_str().unwrap_or("?"),
            c.present,
            c.absent,
            c.unknown
        )
    };
    let mut report = vec![
        format!(
            "Cover draft ({POLICY}) for review window {review_window:?}; technical draft only, no vote."
        ),
        format!(
            "DEM acquisition: {} ({})",
            observed.as_deref().unwrap_or("unknown"),
            date.origin
        ),
        line("Snow/ice", &snow_counts, &manifest["snow_ice"]["kind"]),
        line("Vegetation", &veg_counts, &manifest["vegetation"]["kind"]),
        format!(
            "Sources: WorldCover {}, GLIMS {}, Sentinel-2 {}",
            manifest["sources"]["worldcover"]["status"]
                .as_str()
                .unwrap_or("?"),
            glims_layer.status,
            seasonal.status
        ),
    ];
    if !resolved {
        report.push(
            "SNOW/ICE UNRESOLVED: this tile needs technical attention before v2 qualification."
                .into(),
        );
    }
    report.extend(limitations.iter().map(|l| format!("Limitation: {l}")));
    report.push("Attribution: ".to_string() + &policy.worldcover.citation);
    report.push("Attribution: ".to_string() + &policy.glims.citation);
    report.push("Attribution: ".to_string() + &policy.sentinel2.citation);
    let text = report.join("\n");
    write_new(
        &stage.path().join("report.txt"),
        (text.clone() + "\n").as_bytes(),
    )?;
    io_context("publish cover draft", fs::rename(stage.path(), output))?;
    Ok(format!("{}\n{text}", output.join("cover.json").display()))
}

/// Combine a v1/v2 profile with a cover draft into a new version-2 profile directory.
pub fn merge(profile_path: &Path, cover_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(
        !output.exists(),
        "profile output already exists; choose a new directory",
    )?;
    automatic::check_profile(profile_path)?;
    let mut profile: Value = serde_json::from_slice(&read_small(profile_path)?)?;
    let cover: Value = read_json(cover_path)?;
    ensure(
        cover["kind"] == "cover_draft" && cover["schema_version"] == 1 && cover["policy"] == POLICY,
        "unsupported cover draft",
    )?;
    ensure(
        profile["source_sha256"] == cover["source_sha256"]
            && profile["asset_sha256"] == cover["asset_sha256"],
        "profile and cover draft describe different sources",
    )?;
    let profile_base = profile_path.parent().unwrap_or(Path::new("."));
    let cover_base = cover_path.parent().unwrap_or(Path::new("."));
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create profile parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage merged profile", tempfile::tempdir_in(parent))?;
    let copy = |from: &Path, to: &Path| -> Result<()> {
        if let Some(dir) = to.parent() {
            io_context("create profile subdirectory", fs::create_dir_all(dir))?;
        }
        ensure(
            !to.exists(),
            format!("duplicate profile file {}", to.display()),
        )?;
        io_context("copy profile file", fs::copy(from, to)).map(|_| ())
    };
    let mut files: Vec<String> = profile["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["path"].as_str().map(String::from))
        .collect();
    if profile["water"]["kind"] == "mask" {
        files.extend(profile["water"]["path"].as_str().map(String::from));
    }
    for file in &files {
        copy(&profile_base.join(file), &stage.path().join(file))?;
    }
    let mut evidence = profile["evidence"].as_array().cloned().unwrap_or_default();
    let mut cover_items = cover["evidence"].as_array().cloned().unwrap_or_default();
    let (draft_sha, _) = hash_file(cover_path)?;
    cover_items.push(json!({"id":"cover-draft","path":"cover.json","sha256":draft_sha,
        "description":"Automated cover draft manifest with source status, counts and limitations","source_url":null}));
    for mut item in cover_items {
        let file = item["path"]
            .as_str()
            .ok_or_else(|| AcquisitionError::Invalid("cover evidence lacks a path".into()))?
            .to_string();
        let target = format!("cover/{file}");
        copy(&cover_base.join(&file), &stage.path().join(&target))?;
        item["path"] = json!(target);
        ensure(
            evidence.iter().all(|e| e["id"] != item["id"]),
            "cover evidence ID already used by the profile",
        )?;
        evidence.push(item);
    }
    let descriptor = |key: &str| -> Result<Value> {
        let mut value = cover[key].clone();
        if value["kind"] == "mask" {
            let file = value["path"].as_str().unwrap_or("").to_string();
            let target = format!("cover/{file}");
            copy(&cover_base.join(&file), &stage.path().join(&target))?;
            value["path"] = json!(target);
        }
        if let Some(ids) = value["evidence_ids"].as_array_mut() {
            ids.push(json!("cover-draft"));
        }
        Ok(value)
    };
    profile["snow_ice"] = descriptor("snow_ice")?;
    profile["vegetation"] = descriptor("vegetation")?;
    profile["schema_version"] = json!(2);
    profile["policy"] = json!("supplier_qa_cover_v1");
    profile["evidence"] = json!(evidence);
    let path = stage.path().join("profile.json");
    write_new(&path, &serde_json::to_vec_pretty(&profile)?)?;
    automatic::check_profile(&path)?;
    io_context("publish merged profile", fs::rename(stage.path(), output))?;
    Ok(output.join("profile.json"))
}
