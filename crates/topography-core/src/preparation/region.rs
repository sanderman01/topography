use super::quality;
use crate::{
    acquisition::{
        AcquisitionError, Result, digest, ensure, hash_file, io_context,
        raster::{command, run},
        read_json, read_small, valid_digest, write_new,
    },
    catalog::curation,
    config,
    review::band,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const LAYERS_V1: [&str; 7] = [
    "elevation",
    "validity",
    "provider-validity",
    "water",
    "quality",
    "repairs",
    "features",
];
const LAYERS_V2: [&str; 10] = [
    "elevation",
    "surface-elevation",
    "validity",
    "provider-validity",
    "water",
    "quality",
    "repairs",
    "features",
    "snow-ice",
    "vegetation",
];

const LAYERS_V3: [&str; 11] = [
    "elevation",
    "surface-elevation",
    "validity",
    "provider-validity",
    "water",
    "quality",
    "repairs",
    "features",
    "snow-ice",
    "vegetation",
    "land-cover",
];

fn layers(version: u32) -> &'static [&'static str] {
    match version {
        1 => &LAYERS_V1,
        2 => &LAYERS_V2,
        _ => &LAYERS_V3,
    }
}

fn policy(version: u32) -> &'static str {
    match version {
        1 => "native_preparation_v1",
        2 => "native_ground_cover_v1",
        _ => "native_ground_cover_v2",
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    quality_path: PathBuf,
    curation_record_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Grid {
    origin: [f64; 2],
    size: [u32; 2],
    spacing_m: [f64; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u32,
    inputs: Vec<Input>,
    grid: Grid,
    masked_fill_m: f32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Parent {
    raw_receipt_sha256: String,
    review_sha256: String,
    asset_sha256: String,
    curation_record_sha256: String,
    quality_report_sha256: String,
    primary_class_id: u16,
    destination_window: [u32; 4],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    kind: String,
    policy: String,
    grid: Grid,
    coordinate_system: Value,
    vertical_datum: String,
    vertical_units: String,
    survey: Value,
    masked_fill_m: f32,
    resampling: String,
    parents: Vec<Parent>,
    artifacts: Vec<Artifact>,
    gdal_version: String,
    topography_version: String,
    review_state: String,
    training_admission: String,
}

fn grid_valid(grid: &Grid) -> Result<()> {
    ensure(
        grid.size.iter().all(|n| (1..=4096).contains(n)),
        "prepared grid dimensions must be 1..=4096",
    )?;
    ensure(
        grid.origin.iter().all(|n| n.is_finite())
            && grid
                .spacing_m
                .iter()
                .all(|n| n.is_finite() && (0.5 - 1e-6..=1.0 + 1e-6).contains(n)),
        "prepared grid requires finite origin and native 0.5–1 m spacing",
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn dataset(grid: &Grid, wkt: &str) -> String {
    format!(
        "<VRTDataset rasterXSize=\"{}\" rasterYSize=\"{}\"><SRS>{}</SRS><GeoTransform>{},{},0,{},0,{}</GeoTransform>",
        grid.size[0],
        grid.size[1],
        xml(wkt),
        grid.origin[0],
        grid.spacing_m[0],
        grid.origin[1],
        -grid.spacing_m[1]
    )
}

fn translate(vrt: &Path, output: &Path) -> Result<()> {
    run(command("gdal_translate")
        .args([
            "-q",
            "-of",
            "GTiff",
            "-mask",
            "none",
            "-a_nodata",
            "none",
            "-co",
            "TILED=YES",
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(vrt)
        .arg(output))?;
    Ok(())
}

fn verify_grid(path: &Path, grid: &Grid, crs: &Value, float: bool) -> Result<()> {
    let info: Value = serde_json::from_slice(&run(command("gdalinfo")
        .args(["-json", "-noct", "-norat", "-nogcp", "-if", "GTiff"])
        .arg(path))?)?;
    ensure(
        info["driverShortName"] == "GTiff"
            && info["size"] == json!(grid.size)
            && info["coordinateSystem"] == *crs
            && super::same_transform(
                &info["geoTransform"],
                &json!([
                    grid.origin[0],
                    grid.spacing_m[0],
                    0.0,
                    grid.origin[1],
                    0.0,
                    -grid.spacing_m[1]
                ]),
            )
            && info["bands"].as_array().is_some_and(|b| {
                b.len() == 1 && b[0]["type"] == if float { "Float32" } else { "Byte" }
            })
            && info["bands"][0].get("noDataValue").is_none()
            && (!float || info["bands"][0]["unit"] == "m"),
        "prepared raster grid/type/units mismatch",
    )
}

fn write_band(
    directory: &Path,
    name: &str,
    grid: &Grid,
    wkt: &str,
    bytes: &[u8],
    float: bool,
) -> Result<PathBuf> {
    let sample_bytes = if float { 4 } else { 1 };
    let kind = if float { "Float32" } else { "Byte" };
    let binary = directory.join(format!("{name}.bin"));
    write_new(&binary, bytes)?;
    let vrt = directory.join(format!("{name}.vrt"));
    let body = format!(
        "{}<VRTRasterBand dataType=\"{kind}\" band=\"1\" subClass=\"VRTRawRasterBand\">{}<SourceFilename relativeToVRT=\"1\">{name}.bin</SourceFilename><ImageOffset>0</ImageOffset><PixelOffset>{sample_bytes}</PixelOffset><LineOffset>{}</LineOffset><ByteOrder>LSB</ByteOrder></VRTRasterBand></VRTDataset>",
        dataset(grid, wkt),
        if float { "<UnitType>m</UnitType>" } else { "" },
        grid.size[0] * sample_bytes
    );
    write_new(&vrt, body.as_bytes())?;
    let output = directory.join(format!("{name}.tif"));
    translate(&vrt, &output)?;
    Ok(output)
}

fn layout(grid: &Grid, ready: &[quality::Ready]) -> Result<Vec<[u32; 4]>> {
    let width = grid.size[0] as usize;
    let mut covered = vec![false; width * grid.size[1] as usize];
    let mut windows = Vec::new();
    let first = &ready[0].bundle;
    for item in ready {
        let bundle = &item.bundle;
        ensure(
            bundle.inspection["coordinateSystem"] == first.inspection["coordinateSystem"]
                && bundle.source.spatial.vertical_datum == first.source.spatial.vertical_datum
                && bundle.source.provider == first.source.provider
                && bundle.source.collection_id == first.source.collection_id
                && bundle.source.survey_id == first.source.survey_id,
            "prepared inputs require the same survey, CRS and vertical datum",
        )?;
        let t: [f64; 6] = serde_json::from_value(bundle.inspection["geoTransform"].clone())?;
        ensure(
            t[1] == grid.spacing_m[0] && -t[5] == grid.spacing_m[1] && t[2] == 0.0 && t[4] == 0.0,
            "native input spacing differs from the requested grid; resampling unsupported",
        )?;
        let x = ((t[0] - grid.origin[0]) / grid.spacing_m[0]).round();
        let y = ((grid.origin[1] - t[3]) / grid.spacing_m[1]).round();
        // Division can leave rounding residue on native decimal spacings.
        // Reconstruct the affine origin exactly; do not admit shifted grids.
        ensure(
            x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && y >= 0.0
                && grid.origin[0] + x * grid.spacing_m[0] == t[0]
                && grid.origin[1] - y * grid.spacing_m[1] == t[3],
            "input origin is not aligned to the requested grid",
        )?;
        let [_, _, w, h] = bundle.manifest.window_cells;
        ensure(
            x + f64::from(w) <= f64::from(grid.size[0])
                && y + f64::from(h) <= f64::from(grid.size[1]),
            "input lies outside prepared grid",
        )?;
        let window = [x as u32, y as u32, w, h];
        for row in window[1]..window[1] + h {
            for col in window[0]..window[0] + w {
                let index = row as usize * width + col as usize;
                ensure(
                    !covered[index],
                    "prepared input footprints overlap; no overlap policy implemented",
                )?;
                covered[index] = true;
            }
        }
        windows.push(window);
    }
    ensure(
        covered.iter().all(|c| *c),
        "prepared grid contains uncovered cells",
    )?;
    Ok(windows)
}

/// Compose an exactly aligned native region from currently eligible sources.
/// Prepared output remains unreviewed and excluded from training releases.
pub fn build(runtime: &Path, request_path: &Path) -> Result<PathBuf> {
    let request_bytes = read_small(request_path)?;
    let request: Request = serde_json::from_slice(&request_bytes)?;
    ensure(
        matches!(request.schema_version, 1..=3)
            && !request.inputs.is_empty()
            && request.inputs.len() <= 16,
        "prepared request v1 to v3 requires 1..=16 inputs",
    )?;
    grid_valid(&request.grid)?;
    ensure(
        request.masked_fill_m.is_finite(),
        "masked fill must be finite Float32 metres",
    )?;
    let base = request_path.parent().unwrap_or(Path::new("."));
    let mut ready = Vec::new();
    for input in &request.inputs {
        ready.push(quality::ready(
            runtime,
            &base.join(&input.quality_path),
            &input.curation_record_sha256,
        )?);
    }
    ensure(
        ready
            .iter()
            .all(|item| item.schema_version == request.schema_version),
        "prepared request and all quality inputs must use the same schema version",
    )?;
    let windows = layout(&request.grid, &ready)?;
    let first = &ready[0].bundle;
    let wkt = first.inspection["coordinateSystem"]["wkt"]
        .as_str()
        .ok_or_else(|| AcquisitionError::Invalid("prepared CRS missing".into()))?;
    let root = config::inspect_runtime(runtime)?
        .paths
        .into_iter()
        .find(|(k, _)| *k == "prepared")
        .map(|(_, p)| p)
        .ok_or_else(|| AcquisitionError::Invalid("prepared root missing".into()))?;
    io_context("create prepared root", fs::create_dir_all(&root))?;
    let stage = io_context("stage prepared region", tempfile::tempdir_in(&root))?;
    let scratch = io_context("stage native bands", tempfile::tempdir_in(stage.path()))?;
    let mut parents = Vec::new();
    let layer_names = layers(request.schema_version);
    let mut band_sources: Vec<Vec<PathBuf>> = (0..layer_names.len()).map(|_| Vec::new()).collect();
    for (index, (item, window)) in ready.iter().zip(&windows).enumerate() {
        let [_, _, w, h] = item.bundle.manifest.window_cells;
        let t: [f64; 6] = serde_json::from_value(item.bundle.inspection["geoTransform"].clone())?;
        let grid = Grid {
            origin: [t[0], t[3]],
            size: [w, h],
            spacing_m: request.grid.spacing_m,
        };
        let surface_heights = item.heights.clone();
        let mut heights = surface_heights.clone();
        let mut validity = vec![0; w as usize * h as usize];
        let water: Vec<u8> = item.masks["water"]
            .iter()
            .map(|v| if *v == 2 { 255 } else { 0 })
            .collect();
        for (i, sample) in heights.chunks_exact_mut(4).enumerate() {
            let finite =
                f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]).is_finite();
            if item.masks["water"][i] == 1
                && item.masks["quality"][i] == 1
                && item.provider[i] == 255
                && finite
            {
                validity[i] = 255;
            } else {
                sample.copy_from_slice(&request.masked_fill_m.to_le_bytes());
            }
        }
        let bytes: Vec<&Vec<u8>> = layer_names
            .iter()
            .map(|name| match *name {
                "elevation" => &heights,
                "surface-elevation" => &surface_heights,
                "validity" => &validity,
                "provider-validity" => &item.provider,
                "water" => &water,
                mask => &item.masks[mask],
            })
            .collect();
        for (layer, values) in bytes.into_iter().enumerate() {
            band_sources[layer].push(write_band(
                scratch.path(),
                &format!("input-{index}-{}", layer_names[layer]),
                &grid,
                wkt,
                values,
                layer_names[layer].ends_with("elevation"),
            )?);
        }
        write_new(
            &stage.path().join(format!("quality-{index}.json")),
            &item.report,
        )?;
        parents.push(Parent {
            raw_receipt_sha256: item.bundle.manifest.raw_receipt_sha256.clone(),
            review_sha256: item.check.review_sha256.clone(),
            asset_sha256: item.bundle.manifest.asset_sha256.clone(),
            curation_record_sha256: item.check.curation_record_sha256.clone(),
            quality_report_sha256: item.check.quality_report_sha256.clone(),
            primary_class_id: item
                .check
                .primary_class_id
                .ok_or_else(|| AcquisitionError::Invalid("accepted class missing".into()))?,
            destination_window: *window,
        });
    }
    for (layer, name) in layer_names.iter().enumerate() {
        let float = name.ends_with("elevation");
        let kind = if float { "Float32" } else { "Byte" };
        let mut body = format!(
            "{}<VRTRasterBand dataType=\"{kind}\" band=\"1\">{}",
            dataset(&request.grid, wkt),
            if float { "<UnitType>m</UnitType>" } else { "" }
        );
        for (source, [x, y, w, h]) in band_sources[layer].iter().zip(&windows) {
            body.push_str(&format!("<SimpleSource><SourceFilename relativeToVRT=\"0\">{}</SourceFilename><SourceBand>1</SourceBand><SrcRect xOff=\"0\" yOff=\"0\" xSize=\"{w}\" ySize=\"{h}\"/><DstRect xOff=\"{x}\" yOff=\"{y}\" xSize=\"{w}\" ySize=\"{h}\"/></SimpleSource>", xml(&source.to_string_lossy())));
        }
        body.push_str("</VRTRasterBand></VRTDataset>");
        let vrt = scratch.path().join(format!("region-{name}.vrt"));
        write_new(&vrt, body.as_bytes())?;
        let output = stage.path().join(format!("{name}.tif"));
        translate(&vrt, &output)?;
        verify_grid(
            &output,
            &request.grid,
            &first.inspection["coordinateSystem"],
            float,
        )?;
        // Verify every output cell against each disjoint native source window.
        let all = band(
            &output,
            scratch.path(),
            &format!("verify-{name}"),
            request.grid.size[0] as usize * request.grid.size[1] as usize,
            float,
        )?;
        let stride = if float { 4 } else { 1 };
        for (index, [x, y, w, h]) in windows.iter().enumerate() {
            let original = band(
                &band_sources[layer][index],
                scratch.path(),
                &format!("verify-{index}-{name}"),
                *w as usize * *h as usize,
                float,
            )?;
            for row in 0..*h as usize {
                let begin =
                    ((row + *y as usize) * request.grid.size[0] as usize + *x as usize) * stride;
                ensure(
                    all[begin..begin + *w as usize * stride]
                        == original[row * *w as usize * stride..(row + 1) * *w as usize * stride],
                    "prepared sample mismatch after GDAL composition",
                )?;
            }
        }
    }
    drop(scratch);
    let mut artifacts = Vec::new();
    for name in layer_names
        .iter()
        .map(|n| format!("{n}.tif"))
        .chain((0..parents.len()).map(|i| format!("quality-{i}.json")))
    {
        let path = stage.path().join(&name);
        let (sha256, bytes) = hash_file(&path)?;
        io_context(
            "sync prepared artifact",
            fs::File::open(path).and_then(|f| f.sync_all()),
        )?;
        artifacts.push(Artifact {
            path: name,
            sha256,
            bytes,
        });
    }
    let manifest = Manifest {
        schema_version: request.schema_version,
        kind: "prepared_native_region".into(),
        policy: policy(request.schema_version).into(),
        grid: request.grid,
        coordinate_system: first.inspection["coordinateSystem"].clone(),
        vertical_datum: first
            .source
            .spatial
            .vertical_datum
            .clone()
            .ok_or_else(|| AcquisitionError::Invalid("vertical datum unresolved".into()))?,
        vertical_units: "m".into(),
        survey: json!({"provider": first.source.provider, "collection_id": first.source.collection_id, "survey_id": first.source.survey_id}),
        masked_fill_m: request.masked_fill_m,
        resampling: "none".into(),
        parents,
        artifacts,
        gdal_version: String::from_utf8_lossy(&run(command("gdalinfo").arg("--version"))?)
            .trim()
            .into(),
        topography_version: env!("CARGO_PKG_VERSION").into(),
        review_state: "unreviewed".into(),
        training_admission: "not admitted".into(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    let output = root.join(digest(&bytes));
    let heads: Vec<_> = manifest
        .parents
        .iter()
        .map(|p| (p.review_sha256.clone(), p.curation_record_sha256.clone()))
        .collect();
    ensure(
        read_small(request_path)? == request_bytes,
        "preparation request changed during processing",
    )?;
    for (input, parent) in request.inputs.iter().zip(&manifest.parents) {
        let check = quality::check(
            runtime,
            &base.join(&input.quality_path),
            &input.curation_record_sha256,
        )?;
        ensure(
            check.eligible && check.quality_report_sha256 == parent.quality_report_sha256,
            "eligibility changed during processing",
        )?;
    }
    write_new(&stage.path().join("manifest.json"), &bytes)?;
    if output.exists() {
        verify_files(&output.join("manifest.json"))?;
        ensure(
            read_small(&output.join("manifest.json"))? == bytes,
            "existing prepared manifest differs",
        )?;
    } else {
        io_context("reserve prepared output", fs::create_dir(&output))?;
        io_context("publish prepared region", fs::rename(stage.path(), &output))?;
        io_context(
            "sync prepared root",
            fs::File::open(&root).and_then(|f| f.sync_all()),
        )?;
    }
    let path = output.join("manifest.json");
    curation::register_prepared(runtime, &path, &heads)?;
    Ok(path)
}

pub(crate) fn verify_files(path: &Path) -> Result<()> {
    let bytes = read_small(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| AcquisitionError::Invalid("prepared manifest needs parent".into()))?;
    ensure(
        parent.file_name().and_then(|s| s.to_str()) == Some(digest(&bytes).as_str()),
        "prepared manifest integrity mismatch",
    )?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    grid_valid(&manifest.grid)?;
    ensure(
        matches!(manifest.schema_version, 1..=3)
            && manifest.kind == "prepared_native_region"
            && manifest.policy == policy(manifest.schema_version)
            && manifest.resampling == "none"
            && manifest.review_state == "unreviewed"
            && manifest.training_admission == "not admitted"
            && manifest.masked_fill_m.is_finite()
            && (1..=16).contains(&manifest.parents.len()),
        "unsupported prepared manifest",
    )?;
    let required: BTreeSet<_> = layers(manifest.schema_version)
        .iter()
        .map(|n| format!("{n}.tif"))
        .chain((0..manifest.parents.len()).map(|i| format!("quality-{i}.json")))
        .collect();
    let mut names = BTreeSet::new();
    for a in manifest.artifacts {
        ensure(
            required.contains(&a.path)
                && names.insert(a.path.clone())
                && valid_digest(&a.sha256)
                && a.bytes <= 256 * 1024 * 1024,
            "invalid prepared artifact",
        )?;
        ensure(
            hash_file(&parent.join(&a.path))? == (a.sha256, a.bytes),
            "prepared artifact integrity mismatch",
        )?;
        if a.path.ends_with(".tif") {
            verify_grid(
                &parent.join(&a.path),
                &manifest.grid,
                &manifest.coordinate_system,
                matches!(a.path.as_str(), "elevation.tif" | "surface-elevation.tif"),
            )?;
        }
    }
    ensure(names == required, "prepared artifacts missing")?;
    for (i, source) in manifest.parents.iter().enumerate() {
        ensure(
            [
                &source.raw_receipt_sha256,
                &source.review_sha256,
                &source.asset_sha256,
                &source.curation_record_sha256,
                &source.quality_report_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s))
                && source.primary_class_id > 0
                && digest(&read_small(&parent.join(format!("quality-{i}.json")))?)
                    == source.quality_report_sha256,
            "prepared parent identity mismatch",
        )?;
    }
    Ok(())
}

/// Verify immutable prepared files and report whether registered parents remain current.
pub fn verify(runtime: &Path, path: &Path) -> Result<(bool, String)> {
    verify_files(path)?;
    let manifest: Manifest = read_json(path)?;
    let heads: Vec<_> = manifest
        .parents
        .iter()
        .map(|p| (p.review_sha256.clone(), p.curation_record_sha256.clone()))
        .collect();
    let (registered, current) = curation::prepared_status(runtime, path, &heads)?;
    Ok((
        registered && current,
        serde_json::to_string_pretty(
            &json!({"schema_version": manifest.schema_version, "kind": "prepared_region_status", "manifest_sha256": digest(&read_small(path)?),
        "integrity": "verified", "registered": registered, "current": current, "review_state": "unreviewed", "training_admission": "not admitted"}),
        )?,
    ))
}
