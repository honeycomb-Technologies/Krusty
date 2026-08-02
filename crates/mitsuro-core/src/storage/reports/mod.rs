//! Persistent research reports
//!
//! Stores reports produced by Chat (with research toggle) and Hive sessions.
//! Each report is persisted in SQLite and also written to disk as a Markdown
//! file under `.mitsuro/reports/` within the active workspace when one exists.

mod disk;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub use disk::promote_report_content;
pub use model::{CreateReportInput, Report};
pub use store::ReportStore;
