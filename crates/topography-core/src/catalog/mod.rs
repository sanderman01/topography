//! Versioned source inputs, legends, and a persistent crop-curation journal.

pub mod curation;
mod legend;
pub(crate) mod source;

pub use legend::{LegendSummary, inspect_legend};
pub use source::{SourceSummary, inspect_source};
