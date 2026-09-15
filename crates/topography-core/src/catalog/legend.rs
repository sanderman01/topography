use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;

use crate::contracts::{ContractError, nonempty, read_toml, require, version};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Legend {
    schema_version: u32,
    legend_version: u32,
    status: Status,
    retired_ids: Vec<u16>,
    precedence: Option<Vec<u16>>,
    classes: Vec<Class>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Candidate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Class {
    id: u16,
    name: String,
    color_rgb: [u8; 3],
    flight_experience: String,
}

/// Candidate vocabulary information, without claiming reviewed source coverage.
#[derive(Debug)]
pub struct LegendSummary {
    pub legend_version: u32,
    pub classes: Vec<(u16, String)>,
    pub retired_count: usize,
    pub precedence_configured: bool,
}

/// Check IDs, colors and optional precedence. Historical ID reuse requires review
/// against prior legend versions; a standalone file cannot establish that history.
pub fn inspect_legend(path: &Path) -> Result<LegendSummary, ContractError> {
    let legend: Legend = read_toml(path)?;
    version(legend.schema_version)?;
    let Status::Candidate = legend.status;
    require(
        legend.legend_version > 0,
        "legend_version",
        "must be greater than zero",
    )?;
    require(
        !legend.classes.is_empty(),
        "classes",
        "must contain a candidate class",
    )?;
    let mut retired = BTreeSet::new();
    for id in &legend.retired_ids {
        require(
            *id > 0 && retired.insert(*id),
            "retired_ids",
            "must be unique, positive IDs",
        )?;
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut colors = BTreeSet::new();
    for class in &legend.classes {
        require(
            class.id > 0 && ids.insert(class.id) && !retired.contains(&class.id),
            "classes.id",
            "must be positive, unique, and not retired",
        )?;
        nonempty(&class.name, "classes.name")?;
        require(
            names.insert(class.name.trim().to_lowercase()),
            "classes.name",
            "must be unique",
        )?;
        require(
            colors.insert(class.color_rgb),
            "classes.color_rgb",
            "must be unique",
        )?;
        nonempty(&class.flight_experience, "classes.flight_experience")?;
    }
    if let Some(precedence) = &legend.precedence {
        require(
            precedence.len() == ids.len()
                && precedence.iter().copied().collect::<BTreeSet<_>>() == ids,
            "precedence",
            "must list every active ID exactly once, highest priority first",
        )?;
    }
    Ok(LegendSummary {
        legend_version: legend.legend_version,
        classes: legend
            .classes
            .into_iter()
            .map(|class| (class.id, class.name))
            .collect(),
        retired_count: retired.len(),
        precedence_configured: legend.precedence.is_some(),
    })
}
