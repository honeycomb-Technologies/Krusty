//! Persistent research reports
//!
//! Stores reports produced by Chat (with research toggle) and Hive sessions.
//! Every report is persisted in SQLite. Exact-owner shared reports may also be
//! mirrored as Markdown under `.mitsuro/reports/`; Worker-private reports stay
//! in the ACL-bearing SQLite store because the filesystem mirror has no
//! per-Worker access boundary.

mod disk;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub use disk::promote_report_content;
pub use model::{CreateReportInput, Report, ReportScope};
pub use store::ReportStore;
