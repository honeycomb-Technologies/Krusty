use std::path::{Path, PathBuf};

use krusty_core::storage::WorkspaceMode;

use crate::error::AppError;
use crate::utils::{paths::validate_path_within, text::trimmed_nonempty};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWorkspace {
    pub working_dir: Option<String>,
    pub project_dir: Option<String>,
    pub workspace_mode: WorkspaceMode,
}

pub struct WorkspaceNormalizationPolicy<'a> {
    pub default_mode_without_paths: WorkspaceMode,
    pub selected_fallback_dir: Option<&'a str>,
}

pub fn workspace_base(user_home_dir: Option<&Path>, server_working_dir: &Path) -> PathBuf {
    user_home_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| server_working_dir.to_path_buf())
}

pub fn allowed_root(user_home_dir: Option<&Path>, workspace_base: &Path) -> PathBuf {
    user_home_dir
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| workspace_base.to_path_buf())
}

pub fn normalize_requested_workspace(
    working_dir: Option<&str>,
    project_dir: Option<&str>,
    workspace_mode: Option<WorkspaceMode>,
    policy: WorkspaceNormalizationPolicy<'_>,
) -> NormalizedWorkspace {
    let normalized_working_dir = trimmed_nonempty(working_dir).map(ToOwned::to_owned);
    let normalized_project_dir = trimmed_nonempty(project_dir).map(ToOwned::to_owned);

    let workspace_mode = workspace_mode.unwrap_or_else(|| {
        if normalized_project_dir.is_some() || normalized_working_dir.is_some() {
            WorkspaceMode::Selected
        } else {
            policy.default_mode_without_paths
        }
    });
    let fallback_dir = match workspace_mode {
        WorkspaceMode::Neutral => None,
        WorkspaceMode::Selected | WorkspaceMode::Created => {
            policy.selected_fallback_dir.map(ToOwned::to_owned)
        }
    };

    let working_dir = match workspace_mode {
        WorkspaceMode::Neutral => normalized_working_dir
            .clone()
            .or(normalized_project_dir.clone()),
        WorkspaceMode::Selected | WorkspaceMode::Created => normalized_project_dir
            .clone()
            .or(normalized_working_dir.clone())
            .or(fallback_dir.clone()),
    };
    let project_dir = match workspace_mode {
        WorkspaceMode::Neutral => None,
        WorkspaceMode::Selected | WorkspaceMode::Created => normalized_project_dir
            .or(normalized_working_dir)
            .or(fallback_dir),
    };

    NormalizedWorkspace {
        working_dir,
        project_dir,
        workspace_mode,
    }
}

pub fn normalize_resolved_requested_workspace(
    working_dir: Option<&str>,
    project_dir: Option<&str>,
    workspace_mode: Option<WorkspaceMode>,
    policy: WorkspaceNormalizationPolicy<'_>,
    workspace_base: &Path,
    allowed_root: &Path,
) -> Result<NormalizedWorkspace, AppError> {
    let workspace = normalize_requested_workspace(working_dir, project_dir, workspace_mode, policy);

    Ok(NormalizedWorkspace {
        working_dir: resolve_optional_workspace_path(
            workspace.working_dir.as_deref(),
            workspace_base,
            allowed_root,
        )?,
        project_dir: resolve_optional_workspace_path(
            workspace.project_dir.as_deref(),
            workspace_base,
            allowed_root,
        )?,
        workspace_mode: workspace.workspace_mode,
    })
}

pub fn resolve_scoped_workspace_path(
    requested: Option<&str>,
    workspace_base: &Path,
    allowed_root: &Path,
) -> Result<PathBuf, AppError> {
    let resolved = match trimmed_nonempty(requested) {
        Some(raw) => resolve_workspace_path(workspace_base, raw),
        None => workspace_base.to_path_buf(),
    };
    validate_path_within(allowed_root, &resolved)?;
    Ok(resolved)
}

pub fn resolve_optional_workspace_path(
    requested: Option<&str>,
    workspace_base: &Path,
    allowed_root: &Path,
) -> Result<Option<String>, AppError> {
    trimmed_nonempty(requested)
        .map(|raw| {
            let resolved = resolve_workspace_path(workspace_base, raw);
            validate_path_within(allowed_root, &resolved)?;
            Ok(resolved.to_string_lossy().into_owned())
        })
        .transpose()
}

pub fn resolve_session_working_dir(
    stored: Option<&str>,
    workspace_base: &Path,
    allowed_root: &Path,
) -> Result<PathBuf, AppError> {
    let working_dir = trimmed_nonempty(stored)
        .map(|raw| resolve_workspace_path(workspace_base, raw))
        .unwrap_or_else(|| workspace_base.to_path_buf());
    validate_path_within(allowed_root, &working_dir)?;
    Ok(working_dir)
}

fn resolve_workspace_path(workspace_base: &Path, raw: &str) -> PathBuf {
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        candidate
    } else {
        workspace_base.join(candidate)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        allowed_root, normalize_requested_workspace, normalize_resolved_requested_workspace,
        resolve_optional_workspace_path, resolve_scoped_workspace_path,
        resolve_session_working_dir, workspace_base, NormalizedWorkspace,
        WorkspaceNormalizationPolicy,
    };
    use crate::error::AppError;
    use krusty_core::storage::WorkspaceMode;

    fn create_test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "krusty-workspace-util-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("test dir should exist");
        path
    }

    #[test]
    fn normalize_requested_workspace_defaults_to_neutral_without_paths() {
        let normalized = normalize_requested_workspace(
            None,
            None,
            None,
            WorkspaceNormalizationPolicy {
                default_mode_without_paths: WorkspaceMode::Neutral,
                selected_fallback_dir: None,
            },
        );

        assert_eq!(
            normalized,
            NormalizedWorkspace {
                working_dir: None,
                project_dir: None,
                workspace_mode: WorkspaceMode::Neutral,
            }
        );
    }

    #[test]
    fn normalize_requested_workspace_can_fallback_selected_to_default_dir() {
        let normalized = normalize_requested_workspace(
            None,
            None,
            None,
            WorkspaceNormalizationPolicy {
                default_mode_without_paths: WorkspaceMode::Selected,
                selected_fallback_dir: Some("/workspace"),
            },
        );

        assert_eq!(
            normalized,
            NormalizedWorkspace {
                working_dir: Some("/workspace".to_string()),
                project_dir: Some("/workspace".to_string()),
                workspace_mode: WorkspaceMode::Selected,
            }
        );
    }

    #[test]
    fn normalize_requested_workspace_neutral_mode_drops_project_context() {
        let normalized = normalize_requested_workspace(
            None,
            Some("  /project  "),
            Some(WorkspaceMode::Neutral),
            WorkspaceNormalizationPolicy {
                default_mode_without_paths: WorkspaceMode::Selected,
                selected_fallback_dir: Some("/workspace"),
            },
        );

        assert_eq!(
            normalized,
            NormalizedWorkspace {
                working_dir: Some("/project".to_string()),
                project_dir: None,
                workspace_mode: WorkspaceMode::Neutral,
            }
        );
    }

    #[test]
    fn workspace_base_prefers_user_home_dir() {
        let user_home = std::path::Path::new("/tmp/user-home");
        let server_root = std::path::Path::new("/tmp/server-root");

        assert_eq!(workspace_base(Some(user_home), server_root), user_home);
    }

    #[test]
    fn resolve_optional_workspace_path_resolves_relative_paths_against_workspace_base() {
        let temp_dir = create_test_dir("relative-path");
        let workspace_root = temp_dir.join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        let allowed_root = allowed_root(Some(&workspace_root), &workspace_root);

        let resolved =
            resolve_optional_workspace_path(Some("repo"), &workspace_root, &allowed_root)
                .unwrap_or_else(|_| panic!("relative path should resolve"));

        assert_eq!(
            resolved.as_deref(),
            Some(workspace_root.join("repo").to_string_lossy().as_ref())
        );
    }

    #[test]
    fn normalize_resolved_requested_workspace_rejects_paths_outside_allowed_root() {
        let temp_dir = create_test_dir("outside-root");
        let workspace_root = temp_dir.join("workspace");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");
        let allowed_root = allowed_root(Some(&workspace_root), &workspace_root);

        let result = normalize_resolved_requested_workspace(
            Some(outside_root.to_string_lossy().as_ref()),
            None,
            None,
            WorkspaceNormalizationPolicy {
                default_mode_without_paths: WorkspaceMode::Selected,
                selected_fallback_dir: None,
            },
            &workspace_root,
            &allowed_root,
        );

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn resolve_scoped_workspace_path_defaults_to_workspace_base() {
        let temp_dir = create_test_dir("scoped-default");
        let workspace_root = temp_dir.join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        let allowed_root = allowed_root(Some(&workspace_root), &workspace_root);

        let resolved = resolve_scoped_workspace_path(None, &workspace_root, &allowed_root)
            .unwrap_or_else(|_| panic!("workspace base should resolve"));

        assert_eq!(resolved, workspace_root);
    }

    #[test]
    fn resolve_session_working_dir_supports_legacy_relative_paths() {
        let temp_dir = create_test_dir("legacy-relative");
        let workspace_root = temp_dir.join("workspace");
        let legacy_repo = workspace_root.join("repo");
        std::fs::create_dir_all(&legacy_repo).expect("legacy repo should exist");
        let allowed_root = allowed_root(Some(&workspace_root), &workspace_root);

        let resolved = resolve_session_working_dir(Some("repo"), &workspace_root, &allowed_root)
            .unwrap_or_else(|_| panic!("legacy relative path should resolve"));

        assert_eq!(resolved, legacy_repo);
    }
}
