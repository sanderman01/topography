use std::{env, path::Path, process::ExitCode};

use topography_core::{
    acquisition, catalog, config, contracts::ContractError, datasets, preparation, review,
};

const HELP: &str = "Topography — terrain workshop

Usage: topography [--help | --version]
       topography validate <world|runtime|legend|source> <path>
       topography discover usgs <west,south,east,north> <max-items> <output-dir> [saved-response.json]
       topography discover linz <west,south,east,north> <max-items> <output-dir>
       topography acquire <source.toml> <runtime.toml> <max-bytes>
       topography batch plan <plan.toml> <runtime.toml> <new-run-dir>
       topography batch validate <queue.json>
       topography batch <download|review> <queue.json> <runtime.toml> <run-dir>
       topography gallery build <tiles.json> <legend.toml> <new-output-dir>
       topography gallery serve <runtime.toml> <gallery.json> <port>
       topography import <source.toml> <asset.tif> <runtime.toml> <max-bytes>
       topography verify <raw-receipt.json>
       topography review <raw-receipt.json> <output-dir> <x,y,width,height>
       topography curate draft <runtime.toml> <raw-receipt.json> <review.json> <legend.toml> <draft.json>
       topography curate record <draft.json> <runtime.toml>
       topography curate status <runtime.toml> <review-sha256>
       topography curate check <runtime.toml>
       topography curate purge-rejected <runtime.toml>
       topography curate export <runtime.toml> <review-sha256> <new-output-dir>
       topography cover draft <raw-receipt.json> <review.json> <new-output-dir> [cover-policy.toml]
       topography quality draft <raw-receipt.json> <review.json> <quality.json>
       topography quality assess <quality.json> <new-report.json>
       topography prepare check <runtime.toml> <quality.json> <curation-record-sha256>
       topography prepare build <runtime.toml> <region.json>
       topography prepare verify <runtime.toml> <prepared-manifest.json>
       topography dataset build <runtime.toml> <selection.json> <approval.json>
       topography dataset verify <runtime.toml> <dataset-manifest.json>
       topography dataset fit-build <runtime.toml> <evaluation-manifest.json> <approval.json>
       topography dataset fit-verify <runtime.toml> <fitting-manifest.json>
       topography dataset diffusion <runtime.toml> <gallery.json> <new-output-dir>
       topography world input <runtime.toml> <world.toml> <new-output-dir>
       topography context build <runtime.toml> <plan.toml> <regions-dir>
       topography context serve <regions-dir> <port>
       topography context release <regions-dir> <new-output-dir> [min-per-class]
       topography world sketch <sketch.toml> <new-output-dir>
       topography world preview <world-input-dir> <new-output-dir> [spacing-m]
       topography export heightmap <world-output-dir> <new-output-dir> [intervals] [shared_edge|power_of_two]
       topography export view <export-dir> [port]
       topography evaluate numeric <runtime.toml> <dataset-manifest.json> <python> <new-output-dir>

Options:
  -h, --help     Show this help
  -V, --version  Show the version

Validation reads metadata only. Success does not mean terrain is accepted.
Acquisition/import preserve raw Float32 DEMs; review creates unscaled native crops.
Import, acquire and review print their resulting manifest path. Curation records
explicit actor declarations, evidence and crop decisions. Quality measures masks;
prepare check reports current eligibility (exit 1 if blocked). Prepare build publishes
eligible native regions; outputs remain unreviewed and excluded from training.
Cover draft writes game_cover_v7 water, snow/ice and vegetation masks and the WorldCover
land-cover class layer for a review crop; the gallery worker generates the same layers
automatically.
Processing never creates a human decision.
See docs/plans/terrain-design-master-plan.md for the implementation plan.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();

    match args.as_slice() {
        [] => println!("{HELP}"),
        [flag] if flag == "--help" || flag == "-h" => println!("{HELP}"),
        [flag] if flag == "--version" || flag == "-V" => {
            println!("topography {}", env!("CARGO_PKG_VERSION"));
        }
        [command, kind, path] if command == "validate" => {
            let path = Path::new(path);
            let result = match kind.to_str() {
                Some("world") => validate_world(path),
                Some("runtime") => validate_runtime(path),
                Some("legend") => validate_legend(path),
                Some("source") => validate_source(path),
                _ => return usage_error(),
            };
            if let Err(error) = result {
                eprintln!("Validation failed for {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        }
        [command, provider, bbox, limit, output, saved @ ..]
            if command == "discover" && provider == "usgs" && saved.len() <= 1 =>
        {
            let (Some(bounds), Some(limit)) = (csv::<f64>(bbox), number::<u32>(limit)) else {
                return usage_error();
            };
            return workflow_result(acquisition::discover_usgs(bounds, limit, Path::new(output), saved.first().map(Path::new)).map(|summary| {
                format!("Discovered {} of {} matches{}; known download size {} bytes, {} unknown sizes.\nSources: {}",
                    summary.returned, summary.total_matches, if summary.truncated { " (truncated; narrow the area)" } else { "" },
                    summary.known_download_bytes, summary.unknown_size_assets, Path::new(output).display())
            }));
        }
        [command, action, plan, runtime, output] if command == "batch" && action == "plan" => {
            return workflow_result(
                acquisition::plan(Path::new(plan), Path::new(runtime), Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, provider, bbox, limit, output] if command == "discover" && provider == "linz" => {
            let (Some(bounds), Some(limit)) = (csv::<f64>(bbox), number::<u32>(limit)) else {
                return usage_error();
            };
            return workflow_result(
                acquisition::discover_linz(
                    bounds,
                    limit,
                    Path::new(output),
                    Path::new("data/cache/linz-stac"),
                )
                .map(|s| {
                    format!(
                        "Discovered {} of {} LINZ tiles{}.\nSources: {}",
                        s.returned,
                        s.tiles_in_bounds,
                        if s.truncated {
                            " (truncated; narrow the area)"
                        } else {
                            ""
                        },
                        Path::new(output).display()
                    )
                }),
            );
        }
        [command, source, runtime, limit] if command == "acquire" => {
            let Some(limit) = number::<u64>(limit) else {
                return usage_error();
            };
            return workflow_result(
                acquisition::acquire(Path::new(source), Path::new(runtime), limit)
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, queue] if command == "batch" && action == "validate" => {
            return workflow_result(acquisition::batch::validate(Path::new(queue)));
        }
        [command, action, input, legend, output] if command == "gallery" && action == "build" => {
            return workflow_result(
                review::gallery::build(Path::new(input), Path::new(legend), Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, manifest, port] if command == "gallery" && action == "serve" => {
            let Some(port) = number::<u16>(port) else {
                return usage_error();
            };
            return match review::gallery_server::serve(
                Path::new(runtime),
                Path::new(manifest),
                port,
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => workflow_result(Err(error)),
            };
        }
        [command, action, queue, runtime, output] if command == "batch" => {
            let result = match action.to_str() {
                Some("download") => acquisition::batch::download(
                    Path::new(queue),
                    Path::new(runtime),
                    Path::new(output),
                ),
                Some("review") => acquisition::batch::review(
                    Path::new(queue),
                    Path::new(runtime),
                    Path::new(output),
                ),
                _ => return usage_error(),
            };
            return batch_result(result);
        }
        [command, source, asset, runtime, limit] if command == "import" => {
            let Some(limit) = number::<u64>(limit) else {
                return usage_error();
            };
            return workflow_result(
                acquisition::import(
                    Path::new(source),
                    Path::new(asset),
                    Path::new(runtime),
                    limit,
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, receipt] if command == "verify" => {
            return workflow_result(acquisition::verify(Path::new(receipt)).map(|record| format!(
                "Verified raw receipt: {}\nAsset: {} bytes, SHA-256 {}\nState: raw/unreviewed; no prepared or training admission.",
                record.record_id, record.asset_bytes, record.asset_sha256)));
        }
        [command, receipt, output, window] if command == "review" => {
            let Some(window) = csv::<u32>(window) else {
                return usage_error();
            };
            return workflow_result(
                acquisition::create_review(Path::new(receipt), Path::new(output), window)
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, raw, review, output, policy @ ..]
            if command == "cover" && action == "draft" && policy.len() <= 1 =>
        {
            return workflow_result(acquisition::cover::draft(
                Path::new(raw),
                Path::new(review),
                Path::new(output),
                policy.first().map(Path::new),
            ));
        }
        [command, action, raw, review, output] if command == "quality" && action == "draft" => {
            return workflow_result(
                preparation::draft(Path::new(raw), Path::new(review), Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, input, output] if command == "quality" && action == "assess" => {
            return workflow_result(
                preparation::assess(Path::new(input), Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, input] if command == "prepare" && action == "build" => {
            return workflow_result(
                preparation::build(Path::new(runtime), Path::new(input))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, input] if command == "prepare" && action == "verify" => {
            match preparation::verify(Path::new(runtime), Path::new(input)) {
                Ok((current, report)) => {
                    println!("{report}");
                    return if current {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    };
                }
                Err(error) => return workflow_result(Err(error)),
            }
        }
        [command, action, runtime, dataset, approval]
            if command == "dataset" && action == "fit-build" =>
        {
            return workflow_result(
                datasets::build_fit(Path::new(runtime), Path::new(dataset), Path::new(approval))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, manifest] if command == "dataset" && action == "fit-verify" => {
            return workflow_result(datasets::verify_fit(
                Path::new(runtime),
                Path::new(manifest),
            ));
        }
        [command, action, runtime, selection, approval]
            if command == "dataset" && action == "build" =>
        {
            return workflow_result(
                datasets::build(
                    Path::new(runtime),
                    Path::new(selection),
                    Path::new(approval),
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, world, output, rest @ ..]
            if command == "export" && action == "heightmap" && rest.len() <= 2 =>
        {
            let intervals = match rest
                .first()
                .map(|v| v.to_str().and_then(|s| s.parse().ok()))
            {
                None => 256,
                Some(Some(n)) => n,
                Some(None) => return usage_error(),
            };
            let layout = match rest
                .get(1)
                .map(|v| v.to_str().and_then(topography_core::export::Layout::parse))
            {
                None => topography_core::export::Layout::SharedEdge,
                Some(Some(l)) => l,
                Some(None) => return usage_error(),
            };
            return workflow_result(
                topography_core::export::heightmap(
                    Path::new(world),
                    Path::new(output),
                    intervals,
                    layout,
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, export, rest @ ..]
            if command == "export" && action == "view" && rest.len() <= 1 =>
        {
            let port = match rest
                .first()
                .map(|v| v.to_str().and_then(|s| s.parse().ok()))
            {
                None => 8766,
                Some(Some(p)) => p,
                Some(None) => return usage_error(),
            };
            return workflow_result(
                topography_core::export::view(Path::new(export), port).map(|()| String::new()),
            );
        }
        [command, action, runtime, plan, output] if command == "context" && action == "build" => {
            return workflow_result(topography_core::context::build(
                Path::new(runtime),
                Path::new(plan),
                Path::new(output),
            ));
        }
        [command, action, root, output, rest @ ..]
            if command == "context" && action == "release" && rest.len() <= 1 =>
        {
            let min = match rest.first().map(|v| number::<usize>(v)) {
                None => topography_core::context::VALIDATION_MIN_REGIONS,
                Some(Some(n)) => n,
                Some(None) => return usage_error(),
            };
            return workflow_result(
                topography_core::context::build_release(Path::new(root), Path::new(output), min)
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, root, port] if command == "context" && action == "serve" => {
            let Some(port) = number::<u16>(port) else {
                return usage_error();
            };
            return match topography_core::context::gallery::serve(Path::new(root), port) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => workflow_result(Err(error)),
            };
        }
        [command, action, config, output] if command == "world" && action == "sketch" => {
            return workflow_result(
                topography_core::world::build_sketch(Path::new(config), Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, input, output, rest @ ..]
            if command == "world" && action == "preview" && rest.len() <= 1 =>
        {
            let spacing = match rest
                .first()
                .map(|v| v.to_str().and_then(|s| s.parse().ok()))
            {
                None => 8,
                Some(Some(n)) => n,
                Some(None) => return usage_error(),
            };
            return workflow_result(
                topography_core::world::build_preview(Path::new(input), Path::new(output), spacing)
                    .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, config, output] if command == "world" && action == "input" => {
            return workflow_result(
                topography_core::world::build_input(
                    Path::new(runtime),
                    Path::new(config),
                    Path::new(output),
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, gallery, output]
            if command == "dataset" && action == "diffusion" =>
        {
            return workflow_result(
                datasets::build_diffusion(
                    Path::new(runtime),
                    Path::new(gallery),
                    Path::new(output),
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, manifest] if command == "dataset" && action == "verify" => {
            return workflow_result(datasets::verify(Path::new(runtime), Path::new(manifest)));
        }
        [command, action, runtime, manifest, python, output]
            if command == "evaluate" && action == "numeric" =>
        {
            return workflow_result(
                datasets::evaluate(
                    Path::new(runtime),
                    Path::new(manifest),
                    Path::new(python),
                    Path::new(output),
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, runtime, input, record] if command == "prepare" && action == "check" => {
            let Some(record) = record.to_str() else {
                return usage_error();
            };
            match preparation::check(Path::new(runtime), Path::new(input), record) {
                Ok(result) => {
                    let eligible = result.eligible;
                    let status = workflow_result(result.json());
                    return if eligible { status } else { ExitCode::FAILURE };
                }
                Err(error) => return workflow_result(Err(error)),
            }
        }
        [command, action, runtime, raw, review, legend, output]
            if command == "curate" && action == "draft" =>
        {
            return workflow_result(
                catalog::curation::draft(
                    Path::new(runtime),
                    Path::new(raw),
                    Path::new(review),
                    Path::new(legend),
                    Path::new(output),
                )
                .map(|p| p.display().to_string()),
            );
        }
        [command, action, draft, runtime] if command == "curate" && action == "record" => {
            return workflow_result(catalog::curation::record(
                Path::new(draft),
                Path::new(runtime),
            ));
        }
        [command, action, runtime, review] if command == "curate" && action == "status" => {
            let Some(review) = review.to_str() else {
                return usage_error();
            };
            return workflow_result(catalog::curation::status(Path::new(runtime), review));
        }
        [command, action, runtime] if command == "curate" && action == "check" => {
            return workflow_result(catalog::curation::check(Path::new(runtime)));
        }
        [command, action, runtime] if command == "curate" && action == "purge-rejected" => {
            return workflow_result(catalog::curation::purge_rejected(Path::new(runtime)));
        }
        [command, action, runtime, review, output] if command == "curate" && action == "export" => {
            let Some(review) = review.to_str() else {
                return usage_error();
            };
            return workflow_result(
                catalog::curation::export(Path::new(runtime), review, Path::new(output))
                    .map(|p| p.display().to_string()),
            );
        }
        _ => {
            return usage_error();
        }
    }

    ExitCode::SUCCESS
}

fn usage_error() -> ExitCode {
    eprintln!("Unsupported arguments. Run topography --help for usage.");
    ExitCode::from(2)
}

fn batch_result(result: Result<(bool, String), acquisition::AcquisitionError>) -> ExitCode {
    match result {
        Ok((complete, report)) => {
            println!("{report}");
            if complete {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => workflow_result(Err(error)),
    }
}

fn number<T: std::str::FromStr>(value: &std::ffi::OsStr) -> Option<T> {
    value.to_str()?.parse().ok()
}

fn csv<T: std::str::FromStr>(value: &std::ffi::OsStr) -> Option<[T; 4]> {
    value
        .to_str()?
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<T>, _>>()
        .ok()?
        .try_into()
        .ok()
}

fn workflow_result(result: Result<String, acquisition::AcquisitionError>) -> ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Operation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn validate_world(path: &Path) -> Result<(), ContractError> {
    let world = config::inspect_world(path)?;
    println!("Valid world preset: {} (draft)", world.name);
    println!(
        "Grid: {} x {} cell centers at {} m",
        world.cells[0], world.cells[1], world.spacing_m
    );
    println!("Extent: {} x {} m", world.extent_m[0], world.extent_m[1]);
    println!(
        "One uncompressed Float32 layer: {} bytes; working memory is not estimated",
        world.float32_layer_bytes
    );
    println!(
        "Training tile: {} cells; generation window/stride: {}/{} cells",
        world.training_tile_cells, world.generation_window_cells, world.generation_stride_cells
    );
    println!(
        "Export: {} x {} tiles; full tiles have 1025 x 1025 shared-edge vertices",
        world.export_tiles[0], world.export_tiles[1]
    );
    println!(
        "Final corner tile: {} x {} vertices",
        world.final_tile_vertices[0], world.final_tile_vertices[1]
    );
    println!(
        "Vehicle envelope: {}",
        if world.vehicle_configured {
            "configured; flight suitability still requires review"
        } else {
            "unset; flight suitability cannot be assessed"
        }
    );
    println!(
        "Pilot decisions remain: numeric tolerances, height encoding, class precedence and minimum areas."
    );
    Ok(())
}

fn validate_runtime(path: &Path) -> Result<(), ContractError> {
    let runtime = config::inspect_runtime(path)?;
    println!(
        "Valid runtime preset (draft); GPU VRAM budget: {} GB",
        runtime.gpu_vram_budget_gb
    );
    for (name, path) in runtime.paths {
        println!("{name}: {}", path.display());
    }
    println!("Paths resolved without creating directories or checking available capacity.");
    Ok(())
}

fn validate_legend(path: &Path) -> Result<(), ContractError> {
    let legend = catalog::inspect_legend(path)?;
    println!(
        "Valid candidate legend v{}: {} classes, {} retired IDs",
        legend.legend_version,
        legend.classes.len(),
        legend.retired_count
    );
    for (id, name) in legend.classes {
        println!("{id}: {name}");
    }
    println!(
        "Primary-class precedence: {}",
        if legend.precedence_configured {
            "configured"
        } else {
            "unset"
        }
    );
    println!("Water and validity use separate masks; coverage and feature fidelity need review.");
    Ok(())
}

fn validate_source(path: &Path) -> Result<(), ContractError> {
    let source = catalog::inspect_source(path)?;
    println!(
        "Valid source metadata: {}{}",
        source.record_id,
        if source.synthetic {
            " (synthetic example)"
        } else {
            ""
        }
    );
    if source.training_permission_recorded {
        println!("Training permission: confirmation recorded; evidence has not been verified.");
    } else {
        println!(
            "Training permission: unconfirmed; excluded from prepared and training pools by policy."
        );
    }
    if !source.unresolved_fields.is_empty() {
        println!(
            "Unresolved metadata: {}",
            source.unresolved_fields.join(", ")
        );
    }
    println!(
        "Metadata validation only: assets, checksums, coverage, quality and human acceptance are not verified."
    );
    Ok(())
}
