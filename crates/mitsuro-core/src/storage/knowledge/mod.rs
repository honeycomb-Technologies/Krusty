pub const CURRENT_SNAPSHOT_TITLE: &str = "Current Snapshot";

mod activity;
mod render;
mod snapshot;
#[cfg(test)]
mod tests;

pub use super::reports::promote_report_content;
pub use snapshot::{
    get_current_snapshot, is_current_snapshot, is_current_snapshot_title, refresh_current_snapshot,
    KnowledgeSnapshot,
};
