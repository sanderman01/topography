use std::{env, path::Path, process::ExitCode};

use topography_core::{catalog, config, contracts::ContractError};

const HELP: &str = "Topography — terrain workshop

Usage: topography [--help | --version]
       topography validate <world|runtime|legend|source> <path>

Options:
  -h, --help     Show this help
  -V, --version  Show the version

Validation reads metadata only. Success does not mean terrain is accepted.
Acquisition, processing, training, generation, and export are not implemented.
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
