use crate::storage::refresh_current_snapshot;
use crate::tools::registry::ToolContext;

pub(super) fn refresh_snapshot_warning(
    ctx: &ToolContext,
    db_path: &std::path::Path,
) -> Vec<String> {
    let project_dir = ctx
        .project_dir
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned());

    match refresh_current_snapshot(db_path, project_dir.as_deref(), ctx.user_id.as_deref()) {
        Ok(_) => Vec::new(),
        Err(err) => vec![format!("task updated but snapshot refresh failed: {err}")],
    }
}
