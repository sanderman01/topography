use super::{Grid, Role, checked};
use crate::{
    acquisition::{
        AcquisitionError, Result, digest, ensure, hash_file, io_context, read_json, read_small,
        valid_digest, write_new,
    },
    review::band,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const LAYERS: [&str; 7] = [
    "elevation",
    "validity",
    "provider-validity",
    "water",
    "quality",
    "repairs",
    "features",
];
const COVER_LAYERS: [&str; 3] = ["snow-ice", "vegetation", "surface-elevation"];

#[derive(Serialize)]
struct Payload {
    path: String,
    bytes: usize,
    sha256: String,
}
#[derive(Serialize)]
struct Region {
    prepared_sha256: String,
    group: String,
    role: Role,
    grid: Grid,
    masked_fill_m: f32,
    layers: BTreeMap<String, Payload>,
    #[serde(skip)]
    valid_cells: u64,
    #[serde(skip)]
    water_cells: u64,
    #[serde(skip)]
    snow_ice_cells: Option<u64>,
    #[serde(skip)]
    invalid_land_cells: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResult {
    schema_version: u32,
    kind: String,
    request_sha256: String,
    dataset_sha256: String,
    metrics_sha256: String,
    gpu_used: bool,
    environment: Environment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    python: String,
    numpy: String,
    package_version: String,
    platform: String,
    elapsed_seconds: f64,
    max_self_rss_kib: f64,
    rss_scope: String,
    source_sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Precision {
    max_height_error_m: f64,
    height_rmse_m: f64,
    #[serde(deserialize_with = "nullable_number")]
    max_slope_component_error_m_per_m: Option<f64>,
    #[serde(deserialize_with = "nullable_number")]
    slope_component_rmse_m_per_m: Option<f64>,
    valid_neighbor_pairs: u64,
    nonflat_pairs: u64,
    newly_flat_pairs: u64,
    bit_identical_valid_samples: bool,
}

fn nullable_number<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Option<f64>, D::Error> {
    Option::<f64>::deserialize(decoder)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Representations {
    float32: Precision,
    float16: Precision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionMetrics {
    prepared_sha256: String,
    group: String,
    role: Role,
    valid_cells: u64,
    water_cells: u64,
    #[serde(default)]
    snow_ice_cells: Option<u64>,
    invalid_land_cells: u64,
    representations: Representations,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    schema_version: u32,
    kind: String,
    regions: Vec<RegionMetrics>,
    synthetic: BTreeMap<String, Representations>,
}

fn run_worker(python: &Path, request: &Path, output: &Path, log: &Path) -> Result<()> {
    let file = io_context("create worker log", fs::File::create(log))?;
    let mut child = io_context(
        "start numeric worker",
        Command::new(python)
            .args(["-m", "topography_training.numeric"])
            .arg(request)
            .arg(output)
            .env("OMP_NUM_THREADS", "1")
            .env("OPENBLAS_NUM_THREADS", "1")
            .env("MKL_NUM_THREADS", "1")
            .env("PYTHONNOUSERSITE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(io_context(
                "duplicate worker log",
                file.try_clone(),
            )?))
            .stderr(Stdio::from(file))
            .spawn(),
    )?;
    let start = Instant::now();
    loop {
        let status = io_context("wait for numeric worker", child.try_wait())?;
        if fs::metadata(log)
            .map(|m| m.len() > 1024 * 1024)
            .unwrap_or(false)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AcquisitionError::Invalid(
                "numeric worker log exceeded 1 MiB".into(),
            ));
        }
        if let Some(status) = status {
            if !status.success() {
                let mut message = Vec::new();
                if let Ok(file) = fs::File::open(log) {
                    let _ = file.take(4096).read_to_end(&mut message);
                }
                return Err(AcquisitionError::Invalid(format!(
                    "numeric worker failed: {}",
                    String::from_utf8_lossy(&message)
                )));
            }
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AcquisitionError::Invalid(
                "numeric worker exceeded 600 seconds".into(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn validate_representations(data: &Representations, maximum_pairs: u64) -> Result<()> {
    for p in [&data.float32, &data.float16] {
        let errors_valid = |maximum: f64, rms: f64| {
            maximum.is_finite() && rms.is_finite() && rms >= 0.0 && rms <= maximum
        };
        let slopes_valid = match (
            p.max_slope_component_error_m_per_m,
            p.slope_component_rmse_m_per_m,
        ) {
            (Some(maximum), Some(rms)) => p.valid_neighbor_pairs > 0 && errors_valid(maximum, rms),
            (None, None) => p.valid_neighbor_pairs == 0,
            _ => false,
        };
        ensure(
            errors_valid(p.max_height_error_m, p.height_rmse_m)
                && slopes_valid
                && p.newly_flat_pairs <= p.nonflat_pairs
                && p.nonflat_pairs <= p.valid_neighbor_pairs
                && p.valid_neighbor_pairs <= maximum_pairs
                && (!p.bit_identical_valid_samples
                    || (p.max_height_error_m == 0.0
                        && p.max_slope_component_error_m_per_m.unwrap_or(0.0) == 0.0
                        && p.newly_flat_pairs == 0)),
            "inconsistent numeric errors or neighbor counts",
        )?;
    }
    ensure(
        data.float32.valid_neighbor_pairs == data.float16.valid_neighbor_pairs
            && data.float32.nonflat_pairs == data.float16.nonflat_pairs,
        "numeric representations disagree about input neighbors",
    )
}

fn validate_environment(environment: &Environment) -> Result<()> {
    ensure(
        [
            &environment.python,
            &environment.numpy,
            &environment.package_version,
            &environment.platform,
            &environment.rss_scope,
        ]
        .iter()
        .all(|s| !s.trim().is_empty() && s.len() <= 4096)
            && environment.elapsed_seconds.is_finite()
            && environment.elapsed_seconds >= 0.0
            && environment.max_self_rss_kib.is_finite()
            && environment.max_self_rss_kib > 0.0
            && (2..=3).contains(&environment.source_sha256.len())
            && environment.source_sha256.iter().all(|(name, id)| {
                ["numeric.py", "payload.py", "metrics.py"].contains(&name.as_str())
                    && valid_digest(id)
            })
            && ["numeric.py", "payload.py"].iter().all(|name| {
                environment
                    .source_sha256
                    .get(*name)
                    .is_some_and(|id| valid_digest(id))
            }),
        "incomplete numeric environment provenance",
    )
}

fn validate_metrics(metrics: &Metrics, regions: &[Region], version: u32) -> Result<()> {
    ensure(
        metrics.schema_version == version && metrics.kind == "numeric_representation_metrics",
        "unsupported numeric metrics",
    )?;
    ensure(
        metrics.regions.len() == regions.len(),
        "numeric worker omitted regions",
    )?;
    for (item, expected) in metrics.regions.iter().zip(regions) {
        let [width, height] = expected.grid.size.map(u64::from);
        ensure(
            item.prepared_sha256 == expected.prepared_sha256
                && item.group == expected.group
                && item.role == expected.role
                && item.valid_cells > 0
                && item.valid_cells == expected.valid_cells
                && item.water_cells == expected.water_cells
                && item.snow_ice_cells == expected.snow_ice_cells
                && item.invalid_land_cells == expected.invalid_land_cells,
            "numeric metrics identity or cell counts mismatch",
        )?;
        validate_representations(
            &item.representations,
            (width - 1) * height + (height - 1) * width,
        )?;
    }
    ensure(
        metrics.synthetic.len() == 4
            && ["ramp", "flat", "sharp_ridge", "narrow_ridge"]
                .iter()
                .all(|name| metrics.synthetic.contains_key(*name)),
        "missing synthetic numeric cases",
    )?;
    for data in metrics.synthetic.values() {
        validate_representations(data, 8064)?;
        ensure(
            data.float32.valid_neighbor_pairs == 8064,
            "incomplete synthetic neighbors",
        )?;
    }
    Ok(())
}

/// Verify an evaluation release, decode native payloads, run Python and publish complete results.
pub fn evaluate(runtime: &Path, release: &Path, python: &Path, output: &Path) -> Result<PathBuf> {
    ensure(
        !output.exists(),
        "numeric output already exists; choose a new run directory",
    )?;
    let (dataset_id, selection, members) = checked(runtime, release)?;
    let version = selection.schema_version;
    let output = io_context("resolve numeric output", std::path::absolute(output))?;
    let parent = output
        .parent()
        .ok_or_else(|| AcquisitionError::Invalid("numeric output needs parent".into()))?;
    io_context("create numeric output parent", fs::create_dir_all(parent))?;
    let stage = io_context(
        "stage numeric run",
        tempfile::Builder::new()
            .prefix(".numeric-")
            .tempdir_in(parent),
    )?;
    let scratch = io_context("numeric decode scratch", tempfile::tempdir())?;
    let mut regions = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        let cells = member.grid.size[0] as usize * member.grid.size[1] as usize;
        let mut layers = BTreeMap::new();
        let mut valid_cells = 0;
        let mut water_cells = 0;
        let mut ice_mask = Vec::new();
        let layers_to_read = LAYERS.iter().chain(if version == 2 {
            COVER_LAYERS.iter()
        } else {
            [].iter()
        });
        for &layer in layers_to_read {
            let bytes = band(
                &member.path.with_file_name(format!("{layer}.tif")),
                scratch.path(),
                "decode",
                cells,
                matches!(layer, "elevation" | "surface-elevation"),
            )?;
            if layer == "validity" {
                valid_cells = bytes.iter().filter(|&&value| value == 255).count() as u64;
            } else if layer == "water" {
                water_cells = bytes.iter().filter(|&&value| value == 255).count() as u64;
            } else if layer == "snow-ice" {
                ice_mask.clone_from(&bytes);
            }
            let name = format!("{index:02}-{layer}.bin");
            write_new(&stage.path().join(&name), &bytes)?;
            layers.insert(
                layer.into(),
                Payload {
                    path: name,
                    sha256: digest(&bytes),
                    bytes: bytes.len(),
                },
            );
        }
        ensure(
            valid_cells + water_cells <= cells as u64,
            "inconsistent input cell counts",
        )?;
        let snow_ice_cells =
            (version == 2).then(|| ice_mask.iter().filter(|&&v| v == 2).count() as u64);
        // Snow/ice is ground; products that predate game_cover_v5 count it as invalid land.
        let invalid_land_cells = (cells as u64)
            .checked_sub(valid_cells + water_cells)
            .ok_or_else(|| {
                AcquisitionError::Invalid("overlapping ground and surface cover".into())
            })?;
        regions.push(Region {
            prepared_sha256: member.id,
            group: member.group,
            role: member.role,
            grid: member.grid,
            masked_fill_m: serde_json::from_value(member.manifest["masked_fill_m"].clone())?,
            layers,
            valid_cells,
            water_cells,
            snow_ice_cells,
            invalid_land_cells,
        });
    }
    let request = json!({"schema_version":version,"kind":"numeric_evaluation_job","dataset_sha256":dataset_id,
        "normalization":selection.normalization,"regions":regions});
    let request_bytes = serde_json::to_vec_pretty(&request)?;
    ensure(
        request_bytes.len() <= 1024 * 1024,
        "numeric request exceeds 1 MiB",
    )?;
    let request_path = stage.path().join("request.json");
    write_new(&request_path, &request_bytes)?;
    let worker_output = stage.path().join("worker-output");
    run_worker(
        python,
        &request_path,
        &worker_output,
        &stage.path().join("worker.log"),
    )?;
    let result_path = worker_output.join("result.json");
    let worker: WorkerResult = read_json(&result_path)?;
    let metrics_path = worker_output.join("metrics.json");
    let metrics_bytes = read_small(&metrics_path)?;
    ensure(
        worker.schema_version == version
            && worker.kind == "numeric_worker_result"
            && worker.request_sha256 == digest(&request_bytes)
            && worker.dataset_sha256 == dataset_id
            && worker.metrics_sha256 == digest(&metrics_bytes)
            && !worker.gpu_used,
        "numeric worker completion identity mismatch",
    )?;
    validate_environment(&worker.environment)?;
    validate_metrics(&serde_json::from_slice(&metrics_bytes)?, &regions, version)?;
    ensure(
        read_small(&request_path)? == request_bytes,
        "numeric request changed",
    )?;
    for region in &regions {
        for layer in region.layers.values() {
            ensure(
                hash_file(&stage.path().join(&layer.path))?
                    == (layer.sha256.clone(), layer.bytes as u64),
                "numeric payload changed",
            )?;
        }
    }
    ensure(
        checked(runtime, release)?.0 == dataset_id,
        "dataset changed during numeric job",
    )?;
    io_context(
        "publish metrics in stage",
        fs::rename(&metrics_path, stage.path().join("metrics.json")),
    )?;
    io_context(
        "publish worker result in stage",
        fs::rename(&result_path, stage.path().join("worker-result.json")),
    )?;
    io_context("remove empty worker stage", fs::remove_dir(&worker_output))?;
    let manifest = json!({"schema_version":version,"kind":"verified_numeric_evaluation","dataset_sha256":dataset_id,
        "request_sha256":digest(&request_bytes),"metrics_sha256":digest(&metrics_bytes),
        "worker_result_sha256":hash_file(&stage.path().join("worker-result.json"))?.0,
        "current_at_completion":true,"gpu_used":false,"training_admission":"not admitted",
        "topography_version":env!("CARGO_PKG_VERSION")});
    write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    // Reserving prevents two publishers from silently replacing each other's result.
    io_context("reserve numeric output", fs::create_dir(&output))?;
    io_context("publish numeric run", fs::rename(stage.path(), &output))?;
    io_context(
        "sync numeric output parent",
        fs::File::open(parent).and_then(|f| f.sync_all()),
    )?;
    Ok(output.join("manifest.json"))
}
