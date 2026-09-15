//! Versioned candidate legends and source metadata; persistent storage is pending.

mod legend;
mod source;

pub use legend::{LegendSummary, inspect_legend};
pub use source::{SourceSummary, inspect_source};
