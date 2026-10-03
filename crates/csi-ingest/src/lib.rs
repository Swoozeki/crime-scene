//! Git history ingestion: streaming log parsing, rename tracking, current-tree metrics.

pub mod git;
pub mod ingest;
pub mod log;
pub mod metrics;
#[cfg(any(test, feature = "testutil"))]
pub mod fixture;

pub use git::{CatFile, Git};
pub use ingest::{IngestStats, ingest_repo};
pub use metrics::{analyze_tree, load_functions};

/// Progress callback: receives short human-readable status lines.
pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);
#[cfg(test)]
mod tests;
