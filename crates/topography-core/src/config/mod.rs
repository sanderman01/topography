//! Configuration loading, physical grid consistency, and schema validation.

mod runtime;
mod world;

pub use runtime::{RuntimeSummary, inspect_runtime};
pub use world::{WorldSummary, inspect_world};

use serde::Deserialize;

/// Version-one presets remain pilot proposals, even when structurally valid.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PresetStatus {
    Draft,
}
