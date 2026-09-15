use std::{env, process::ExitCode};

const HELP: &str = "Topography — terrain workshop

Usage: topography [--help | --version]

Options:
  -h, --help     Show this help
  -V, --version  Show the version

This is the project scaffold. Terrain commands are not implemented yet.
See docs/plans/terrain-design-master-plan.md for the implementation plan.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();

    match args.as_slice() {
        [] => println!("{HELP}"),
        [flag] if flag == "--help" || flag == "-h" => println!("{HELP}"),
        [flag] if flag == "--version" || flag == "-V" => {
            println!("topography {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("Unsupported arguments. Run topography --help for usage.");
            return ExitCode::from(2);
        }
    }

    ExitCode::SUCCESS
}
