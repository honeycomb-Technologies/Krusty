//! Per-attempt isolated workspaces for parallel builders.
//!
//! Parallel write children never mutate the authoritative workspace directly.
//! Each receives the same captured source snapshot in a detached worktree;
//! synthesis applies their patches back in deterministic task order.

use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use super::build_context::SharedBuildContext;
use super::{SubAgentResult, SubAgentTermination};

#[derive(Debug, Clone)]
pub(crate) struct IsolatedBuildWorkspace {
    pub task_id: String,
    pub root: PathBuf,
    pub project_dir: PathBuf,
    pub working_dir: PathBuf,
    pub baseline_commit: String,
}

impl IsolatedBuildWorkspace {
    pub(crate) fn command_environment(&self) -> std::collections::BTreeMap<String, String> {
        [
            ("CARGO_HOME", self.root.join(".cargo-home")),
            ("TMPDIR", self.root.join(".mitsuro/tmp")),
            ("XDG_CACHE_HOME", self.root.join(".mitsuro/cache")),
            ("npm_config_cache", self.root.join(".mitsuro/npm")),
        ]
        .into_iter()
        .map(|(key, path)| (key.to_string(), path.display().to_string()))
        .collect()
    }
}

#[derive(Debug)]
pub struct BuildIsolationSet {
    group_id: String,
    repo_root: PathBuf,
    base_dir: PathBuf,
    workspaces: Vec<IsolatedBuildWorkspace>,
    backend: IsolationBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsolationBackend {
    GitWorktree,
    SnapshotRepository,
}

const SNAPSHOT_BASELINE_PREFIX: &str = "snapshot:";
const SNAPSHOT_REPOSITORY_NAME: &str = ".snapshot.git";
const GENERATED_INTEGRATION_EXCLUDES: &[&str] =
    &["target", ".cargo-home", "node_modules", ".mitsuro"];
// Parallel builders commonly run the same compiler against one captured
// baseline. These outputs are useful for local validation but are not source
// authority and will conflict even when the builders' declared source scopes
// are disjoint. Unlike the cache/runtime paths above, a final verifier may
// explicitly publish a distributable directory through generated_artifacts.
const RESTAGEABLE_BUILD_OUTPUT_DIRS: &[&str] = &["dist"];
const RESTAGEABLE_BUILD_OUTPUT_GLOBS: &[&str] = &["*.tsbuildinfo"];
const DELEGATED_ENVIRONMENT_DIRS: &[&str] = &[
    ".cargo-home",
    ".mitsuro/tmp",
    ".mitsuro/cache",
    ".mitsuro/npm",
];
const WRITABLE_NODE_MODULES_ENTRIES: &[&str] = &[".cache", ".vite", ".vite-temp"];

/// Cross-process ownership for the narrow interval between preparing a Hive
/// batch's worktrees and persisting its immutable delegation group. The lock
/// file lives in one of a fixed number of slots in the UID-private isolation
/// root; file locking, not file existence, carries ownership.
#[derive(Debug)]
pub(crate) struct BuildIsolationMaterializationGuard {
    group_id: String,
    _lock: fs::File,
}

impl BuildIsolationMaterializationGuard {
    pub async fn acquire(group_id: String) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
            validate_group_id(&group_id)?;
            let lock_path = isolation_root()?.join(format!(
                ".materialize-{:02}.lock",
                materialization_lock_slot(&group_id)
            ));
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)
                .with_context(|| format!("open materialization lock {}", lock_path.display()))?;
            FileExt::lock_exclusive(&lock).context("lock Hive batch materialization")?;
            Ok(Self {
                group_id,
                _lock: lock,
            })
        })
        .await
        .context("Hive materialization lock worker panicked")?
    }

    /// Remove only a deterministic abandoned preparation after the caller has
    /// proved that no durable group owns it. Any unexpected entry, symlink, or
    /// worktree belonging to another repository fails closed and is retained.
    pub async fn remove_abandoned_preparation(
        &self,
        project_dir: PathBuf,
        expected_task_count: usize,
    ) -> Result<bool> {
        let group_id = self.group_id.clone();
        tokio::task::spawn_blocking(move || {
            let base_dir = isolation_root()?.join(&group_id);
            if !base_dir.exists() {
                return Ok(false);
            }
            let metadata = fs::symlink_metadata(&base_dir)?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "abandoned isolation batch is not an owned directory"
            );
            let allowed = (0..expected_task_count)
                .map(|ordinal| format!("task-{ordinal:04}"))
                .chain([SNAPSHOT_REPOSITORY_NAME.to_string()])
                .collect::<std::collections::HashSet<_>>();
            let snapshot_repository = base_dir.join(SNAPSHOT_REPOSITORY_NAME);
            let backend = if snapshot_repository.exists() {
                IsolationBackend::SnapshotRepository
            } else {
                IsolationBackend::GitWorktree
            };
            let repo = match backend {
                IsolationBackend::GitWorktree => resolve_repo(&project_dir)?,
                IsolationBackend::SnapshotRepository => snapshot_repository.clone(),
            };
            let expected_common_dir = git_common_dir(&repo)?;
            let mut roots = Vec::new();
            for entry in fs::read_dir(&base_dir)? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("abandoned isolation entry is not UTF-8"))?;
                ensure!(
                    allowed.contains(&name),
                    "abandoned isolation batch contains an unexpected entry"
                );
                if name == SNAPSHOT_REPOSITORY_NAME {
                    continue;
                }
                let entry_metadata = fs::symlink_metadata(entry.path())?;
                ensure!(
                    entry_metadata.is_dir() && !entry_metadata.file_type().is_symlink(),
                    "abandoned isolation entry is not a directory"
                );
                roots.push(entry.path());
            }
            roots.sort();
            for root in roots {
                if fs::read_dir(&root)?.next().is_none() {
                    fs::remove_dir(&root)?;
                    continue;
                }
                ensure!(
                    resolve_repo(&root)? == root,
                    "abandoned isolation entry is not a worktree root"
                );
                ensure!(
                    git_common_dir(&root)? == expected_common_dir,
                    "abandoned isolation worktree belongs to another repository"
                );
                remove_worktree(&repo, &root)?;
            }
            if backend == IsolationBackend::SnapshotRepository {
                fs::remove_dir_all(&snapshot_repository).with_context(|| {
                    format!(
                        "remove abandoned snapshot repository {}",
                        snapshot_repository.display()
                    )
                })?;
            }
            fs::remove_dir(&base_dir).with_context(|| {
                format!("remove abandoned isolation batch {}", base_dir.display())
            })?;
            Ok(true)
        })
        .await
        .context("abandoned Hive preparation recovery panicked")?
    }
}

impl BuildIsolationSet {
    /// Materialize one isolated workspace per task. Established Git projects
    /// use detached worktrees; unborn and non-Git projects use a private bare
    /// snapshot repository without mutating the authoritative workspace.
    pub async fn prepare(
        project_dir: PathBuf,
        working_dir: PathBuf,
        group_id: String,
        task_ids: Vec<String>,
    ) -> Result<Option<Self>> {
        tokio::task::spawn_blocking(move || {
            Self::prepare_blocking(&project_dir, &working_dir, &group_id, &task_ids)
        })
        .await
        .context("parallel build isolation worker panicked")?
    }

    fn prepare_blocking(
        project_dir: &Path,
        working_dir: &Path,
        group_id: &str,
        task_ids: &[String],
    ) -> Result<Option<Self>> {
        let project_dir = project_dir
            .canonicalize()
            .context("canonicalize delegated project workspace")?;
        let working_dir = working_dir
            .canonicalize()
            .context("canonicalize delegated working directory")?;
        let discovered_repo = command_output(Command::new("git").args([
            "-C",
            &project_dir.display().to_string(),
            "rev-parse",
            "--show-toplevel",
        ]));
        let (repo, backend) = match discovered_repo {
            Ok(output) => {
                let repo = PathBuf::from(String::from_utf8(output.stdout)?.trim());
                let has_head = command_output(Command::new("git").args([
                    "-C",
                    &repo.display().to_string(),
                    "rev-parse",
                    "--verify",
                    "HEAD",
                ]))
                .is_ok();
                if has_head && !path_is_ignored_by_repository(&repo, &project_dir)? {
                    (repo, IsolationBackend::GitWorktree)
                } else {
                    (project_dir.clone(), IsolationBackend::SnapshotRepository)
                }
            }
            Err(error) if error.to_string().contains("not a git repository") => {
                (project_dir.clone(), IsolationBackend::SnapshotRepository)
            }
            Err(error) => return Err(error).context("resolve parallel build repository"),
        };
        ensure!(
            working_dir.starts_with(&repo),
            "working directory is outside repository root"
        );
        let relative_project_dir = project_dir
            .strip_prefix(&repo)
            .context("resolve delegated project directory inside repository")?;
        let relative_working_dir = working_dir
            .strip_prefix(&repo)
            .context("resolve delegated working directory inside repository")?;
        validate_group_id(group_id)?;
        ensure!(
            !integration_marker_for(&repo, group_id, backend)?.exists(),
            "delegation batch already has a durable integration marker"
        );
        let base_dir = isolation_root()?.join(group_id);
        ensure!(
            !base_dir.exists(),
            "delegation isolation directory already exists"
        );
        fs::create_dir_all(&base_dir)
            .with_context(|| format!("create isolation root {}", base_dir.display()))?;

        let (source_patch, untracked, snapshot_commit) = match backend {
            IsolationBackend::GitWorktree => (
                command_output(Command::new("git").args([
                    "-C",
                    &repo.display().to_string(),
                    "diff",
                    "--binary",
                    "HEAD",
                ]))?
                .stdout,
                command_output(Command::new("git").args([
                    "-C",
                    &repo.display().to_string(),
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ]))?
                .stdout,
                None,
            ),
            IsolationBackend::SnapshotRepository => (
                Vec::new(),
                Vec::new(),
                Some(create_snapshot_repository(&repo, &base_dir)?),
            ),
        };

        let mut workspaces: Vec<IsolatedBuildWorkspace> = Vec::new();
        for (ordinal, task_id) in task_ids.iter().enumerate() {
            let root = base_dir.join(format!("task-{ordinal:04}"));
            let prepared = (|| -> Result<IsolatedBuildWorkspace> {
                match backend {
                    IsolationBackend::GitWorktree => {
                        prepare_one(&repo, &root, &source_patch, &untracked)?;
                    }
                    IsolationBackend::SnapshotRepository => prepare_snapshot_worktree(
                        &base_dir.join(SNAPSHOT_REPOSITORY_NAME),
                        &root,
                        snapshot_commit
                            .as_deref()
                            .context("snapshot baseline commit is missing")?,
                    )?,
                }
                let baseline_commit = String::from_utf8(
                    command_output(Command::new("git").args([
                        "-C",
                        &root.display().to_string(),
                        "rev-parse",
                        "HEAD",
                    ]))?
                    .stdout,
                )?
                .trim()
                .to_string();
                let baseline_commit = if backend == IsolationBackend::SnapshotRepository {
                    format!("{SNAPSHOT_BASELINE_PREFIX}{baseline_commit}")
                } else {
                    baseline_commit
                };
                for relative in DELEGATED_ENVIRONMENT_DIRS {
                    fs::create_dir_all(root.join(relative)).with_context(|| {
                        format!("create delegated environment directory {relative}")
                    })?;
                }
                link_authoritative_node_modules(&repo, &project_dir, &working_dir, &root)?;
                Ok(IsolatedBuildWorkspace {
                    task_id: task_id.clone(),
                    project_dir: root.join(relative_project_dir),
                    working_dir: root.join(relative_working_dir),
                    root: root.clone(),
                    baseline_commit,
                })
            })();
            match prepared {
                Ok(workspace) => workspaces.push(workspace),
                Err(error) => {
                    let cleanup_repo = if backend == IsolationBackend::SnapshotRepository {
                        base_dir.join(SNAPSHOT_REPOSITORY_NAME)
                    } else {
                        repo.clone()
                    };
                    let _ = remove_worktree(&cleanup_repo, &root);
                    for workspace in &workspaces {
                        let _ = remove_worktree(&cleanup_repo, &workspace.root);
                    }
                    let _ = fs::remove_dir_all(&base_dir);
                    return Err(error);
                }
            }
        }

        Ok(Some(Self {
            group_id: group_id.to_string(),
            repo_root: repo,
            base_dir,
            workspaces,
            backend,
        }))
    }

    /// Reconstruct an isolation set solely from immutable durable task
    /// contracts. Every persisted root must match the deterministic ordinal
    /// path; arbitrary database paths can never become cleanup or integration
    /// targets.
    pub async fn restore(
        project_dir: PathBuf,
        group_id: String,
        workspaces: Vec<(String, PathBuf, String)>,
    ) -> Result<Self> {
        tokio::task::spawn_blocking(move || {
            validate_group_id(&group_id)?;
            let backend = if workspaces
                .iter()
                .all(|(_, _, baseline)| baseline.starts_with(SNAPSHOT_BASELINE_PREFIX))
            {
                IsolationBackend::SnapshotRepository
            } else {
                ensure!(
                    workspaces
                        .iter()
                        .all(|(_, _, baseline)| !baseline.starts_with(SNAPSHOT_BASELINE_PREFIX)),
                    "durable isolation batch mixes workspace backends"
                );
                IsolationBackend::GitWorktree
            };
            let repo = match backend {
                IsolationBackend::GitWorktree => resolve_repo(&project_dir)?,
                IsolationBackend::SnapshotRepository => project_dir
                    .canonicalize()
                    .context("canonicalize snapshot project workspace")?,
            };
            let canonical_project = project_dir
                .canonicalize()
                .context("canonicalize restored project workspace")?;
            let relative_project_dir = canonical_project
                .strip_prefix(&repo)
                .context("resolve restored project directory inside repository")?
                .to_path_buf();
            let base_dir = isolation_root()?.join(&group_id);
            let marker = integration_marker_for(&repo, &group_id, backend)?;
            let already_integrated = integration_marker_is_valid(&marker, &group_id)?;
            if !base_dir.exists() {
                ensure!(
                    already_integrated,
                    "durable isolation root is missing before patch integration"
                );
                return Ok(Self {
                    group_id,
                    repo_root: repo,
                    base_dir,
                    workspaces: Vec::new(),
                    backend,
                });
            }
            let base_metadata = fs::symlink_metadata(&base_dir)?;
            ensure!(
                base_metadata.is_dir() && !base_metadata.file_type().is_symlink(),
                "durable isolation root is not an owned directory"
            );
            let expected_common_dir = match backend {
                IsolationBackend::GitWorktree => git_common_dir(&repo)?,
                IsolationBackend::SnapshotRepository => {
                    let snapshot_repository = base_dir.join(SNAPSHOT_REPOSITORY_NAME);
                    ensure!(
                        snapshot_repository.exists(),
                        "durable snapshot repository is missing before integration"
                    );
                    git_common_dir(&snapshot_repository)?
                }
            };
            let allowed = (0..workspaces.len())
                .map(|ordinal| format!("task-{ordinal:04}"))
                .chain(
                    (backend == IsolationBackend::SnapshotRepository)
                        .then(|| SNAPSHOT_REPOSITORY_NAME.to_string()),
                )
                .collect::<std::collections::HashSet<_>>();
            for entry in fs::read_dir(&base_dir)? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("durable isolation entry is not UTF-8"))?;
                ensure!(
                    allowed.contains(&name),
                    "durable isolation batch contains an unexpected entry"
                );
            }
            let mut restored = Vec::with_capacity(workspaces.len());
            for (ordinal, (task_id, root, baseline_commit)) in workspaces.into_iter().enumerate() {
                let expected = base_dir.join(format!("task-{ordinal:04}"));
                ensure!(root == expected, "durable isolation root escaped its batch");
                let raw_baseline = raw_baseline_commit(&baseline_commit)?;
                if !root.exists() {
                    ensure!(
                        already_integrated,
                        "durable isolation worktree is missing before patch integration"
                    );
                    continue;
                }
                let root_metadata = fs::symlink_metadata(&root)?;
                ensure!(
                    root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
                    "durable isolation worktree is not an owned directory"
                );
                let resolved = resolve_repo(&root)?;
                ensure!(
                    resolved == root,
                    "durable isolation root is not a worktree root"
                );
                ensure!(
                    git_common_dir(&root)? == expected_common_dir,
                    "durable isolation worktree belongs to another repository"
                );
                command_output(Command::new("git").args([
                    "-C",
                    &root.display().to_string(),
                    "cat-file",
                    "-e",
                    &format!("{raw_baseline}^{{commit}}"),
                ]))
                .context("durable isolation baseline is not a repository commit")?;
                restored.push(IsolatedBuildWorkspace {
                    task_id,
                    project_dir: root.join(&relative_project_dir),
                    working_dir: root.join(&relative_project_dir),
                    root,
                    baseline_commit,
                });
            }
            Ok(Self {
                group_id,
                repo_root: repo,
                base_dir,
                workspaces: restored,
                backend,
            })
        })
        .await
        .context("restore build isolation worker panicked")?
    }

    /// Roll back a prepared batch that never became durable authority.
    pub async fn discard(self) -> Result<()> {
        tokio::task::spawn_blocking(move || {
            let mut first_error = None;
            let cleanup_repo = match self.backend {
                IsolationBackend::GitWorktree => self.repo_root.clone(),
                IsolationBackend::SnapshotRepository => {
                    self.base_dir.join(SNAPSHOT_REPOSITORY_NAME)
                }
            };
            for workspace in &self.workspaces {
                if let Err(error) = remove_worktree(&cleanup_repo, &workspace.root) {
                    first_error.get_or_insert(error);
                }
            }
            if self.backend == IsolationBackend::SnapshotRepository && cleanup_repo.exists() {
                if let Err(error) = fs::remove_dir_all(&cleanup_repo) {
                    first_error.get_or_insert(error.into());
                }
            }
            if self.base_dir.exists() {
                if let Err(error) = fs::remove_dir(&self.base_dir).with_context(|| {
                    format!("remove unused isolation root {}", self.base_dir.display())
                }) {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            Ok(())
        })
        .await
        .context("discard build isolation worker panicked")?
    }

    pub(crate) fn workspaces(&self) -> &[IsolatedBuildWorkspace] {
        &self.workspaces
    }

    pub(crate) fn group_id(&self) -> &str {
        &self.group_id
    }

    pub(crate) fn task_ids(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .map(|workspace| workspace.task_id.clone())
            .collect()
    }

    /// Apply child patches to the authoritative workspace in task order.
    /// Conflicted worktrees are retained for recovery and their task result is
    /// downgraded before aggregate synthesis.
    pub async fn integrate(self, results: Vec<SubAgentResult>) -> Vec<SubAgentResult> {
        self.integrate_with_fence(results, None, None).await
    }

    /// Apply child patches and project their authoritative diff statistics into
    /// the aggregate build context. Child tool counters belong to isolated
    /// worktrees, so the integration boundary is the source of truth.
    pub async fn integrate_recording(
        self,
        results: Vec<SubAgentResult>,
        context: Arc<SharedBuildContext>,
    ) -> Vec<SubAgentResult> {
        self.integrate_with_fence(results, None, Some(context))
            .await
    }

    /// Recovery-only integration under a caller-provided durable owner fence.
    /// The fence runs while the repository integration lock is held and
    /// immediately before the idempotent patch/marker transaction.
    pub async fn integrate_recovered(
        self,
        results: Vec<SubAgentResult>,
        owner_fence: std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Vec<SubAgentResult> {
        self.integrate_with_fence(results, Some(owner_fence), None)
            .await
    }

    async fn integrate_with_fence(
        self,
        results: Vec<SubAgentResult>,
        owner_fence: Option<std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>>,
        build_context: Option<Arc<SharedBuildContext>>,
    ) -> Vec<SubAgentResult> {
        let fallback = results
            .iter()
            .map(|result| (result.task_id.clone(), result.agent_name.clone()))
            .collect::<Vec<_>>();
        let mut owned_results = results;
        match tokio::task::spawn_blocking(move || {
            self.integrate_blocking(
                &mut owned_results,
                owner_fence.as_deref(),
                build_context.as_deref(),
            )
        })
        .await
        {
            Ok(results) => results,
            Err(error) => fallback
                .into_iter()
                .map(|(task_id, agent_name)| SubAgentResult {
                    task_id,
                    agent_name,
                    delegated_run_id: None,
                    success: false,
                    output: String::new(),
                    files_examined: Vec::new(),
                    duration_ms: 0,
                    turns_used: 0,
                    error: Some(format!("Parallel build integrator panicked: {error}")),
                    termination: SubAgentTermination::Failed,
                    policy_violations: Vec::new(),
                    evidence: Default::default(),
                    background_processes: Vec::new(),
                })
                .collect(),
        }
    }

    fn integrate_blocking(
        self,
        results: &mut [SubAgentResult],
        owner_fence: Option<&(dyn Fn() -> Result<()> + Send + Sync)>,
        build_context: Option<&SharedBuildContext>,
    ) -> Vec<SubAgentResult> {
        if let Err(error) = self.integrate_batch(results, owner_fence, build_context) {
            for workspace in &self.workspaces {
                if let Some(result) = results.iter_mut().find(|result| {
                    result.task_id == workspace.task_id
                        && result.is_eligible_for_isolated_integration()
                }) {
                    result.success = false;
                    result.termination = SubAgentTermination::Failed;
                    result.error = Some(format!(
                        "Isolated patch integration failed; recovery workspace retained at {}: {error}",
                        workspace.root.display()
                    ));
                }
            }
            return results.to_vec();
        }
        for workspace in &self.workspaces {
            let Some(result) = results.iter_mut().find(|result| {
                result.task_id == workspace.task_id && result.is_eligible_for_isolated_integration()
            }) else {
                continue;
            };
            rewrite_result_paths(result, &workspace.root, &self.repo_root);
            let cleanup_repo = match self.backend {
                IsolationBackend::GitWorktree => self.repo_root.clone(),
                IsolationBackend::SnapshotRepository => {
                    // All private snapshot worktrees share this bare repository.
                    // Retain it whenever a failed workspace remains recoverable.
                    self.base_dir.join(SNAPSHOT_REPOSITORY_NAME)
                }
            };
            if let Err(error) = remove_worktree(&cleanup_repo, &workspace.root) {
                result.error = Some(format!(
                    "Build patch integrated, but isolated workspace cleanup failed at {}: {error}",
                    workspace.root.display()
                ));
            }
        }
        if self.backend == IsolationBackend::SnapshotRepository
            && self
                .workspaces
                .iter()
                .all(|workspace| !workspace.root.exists())
        {
            let _ = fs::remove_dir_all(self.base_dir.join(SNAPSHOT_REPOSITORY_NAME));
        }
        let _ = fs::remove_dir(&self.base_dir);
        results.to_vec()
    }

    fn integrate_batch(
        &self,
        results: &[SubAgentResult],
        owner_fence: Option<&(dyn Fn() -> Result<()> + Send + Sync)>,
        build_context: Option<&SharedBuildContext>,
    ) -> Result<()> {
        let lock_path = match self.backend {
            IsolationBackend::GitWorktree => {
                let common_dir = git_common_dir(&self.repo_root)?;
                fs::create_dir_all(&common_dir)?;
                common_dir.join("mitsuro-delegation-integration.lock")
            }
            IsolationBackend::SnapshotRepository => isolation_root()?.join(format!(
                ".integration-{:02}.lock",
                materialization_lock_slot(&self.group_id)
            )),
        };
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open integration lock {}", lock_path.display()))?;
        FileExt::lock_exclusive(&lock).context("lock delegated integration")?;

        if let Some(owner_fence) = owner_fence {
            owner_fence().context("detached build recovery lost its durable owner fence")?;
        }
        let marker = integration_marker_for(&self.repo_root, &self.group_id, self.backend)?;
        if integration_marker_is_valid(&marker, &self.group_id)? {
            return Ok(());
        }

        const MAX_COMBINED_PATCH_BYTES: usize = 64 * 1024 * 1024;
        let mut combined = Vec::new();
        let mut integrated_changes = Vec::new();
        for workspace in &self.workspaces {
            let Some(result) = results.iter().find(|result| {
                result.task_id == workspace.task_id && result.is_eligible_for_isolated_integration()
            }) else {
                continue;
            };
            let mut stage = Command::new("git");
            stage.args(["-C", &workspace.root.display().to_string(), "add", "-A"]);
            command_output(&mut stage)?;
            let mut unstage_generated = Command::new("git");
            unstage_generated.args([
                "-C",
                &workspace.root.display().to_string(),
                "reset",
                "-q",
                "--",
            ]);
            add_integration_excluded_pathspecs(&mut unstage_generated);
            command_output(&mut unstage_generated)?;
            stage_declared_generated_artifacts(workspace, result)?;
            let baseline_commit = raw_baseline_commit(&workspace.baseline_commit)?;
            let patch = command_output(Command::new("git").args([
                "-C",
                &workspace.root.display().to_string(),
                "diff",
                "--cached",
                "--binary",
                baseline_commit,
            ]))?
            .stdout;
            ensure!(
                !patch.is_empty() || result.evidence.mutations == 0,
                "isolated task {} reported {} mutation(s), but its integration patch was empty",
                workspace.task_id,
                result.evidence.mutations
            );
            if build_context.is_some() {
                let numstat = command_output(Command::new("git").args([
                    "-C",
                    &workspace.root.display().to_string(),
                    "diff",
                    "--cached",
                    "--no-renames",
                    "--numstat",
                    "-z",
                    baseline_commit,
                ]))?
                .stdout;
                integrated_changes.extend(parse_integration_numstat(&workspace.task_id, &numstat));
            }
            ensure!(
                combined.len().saturating_add(patch.len()) <= MAX_COMBINED_PATCH_BYTES,
                "delegated integration patch exceeds the bounded size"
            );
            if patch.is_empty() {
                continue;
            }
            combined.extend_from_slice(&patch);
            if !combined.ends_with(b"\n") {
                combined.push(b'\n');
            }
        }

        if let Some(owner_fence) = owner_fence {
            // Patch construction can be expensive. Re-prove both durable
            // owners at the last safe boundary before authoritative writes.
            owner_fence().context("detached build recovery lost its durable publication fence")?;
        }
        if !combined.is_empty() {
            let mut check = self.integration_apply_command();
            check.args(["apply", "--check", "--whitespace=nowarn", "-"]);
            if command_with_stdin(&mut check, &combined).is_ok() {
                let mut apply = self.integration_apply_command();
                apply.args(["apply", "--whitespace=nowarn", "-"]);
                command_with_stdin(&mut apply, &combined)?;
            } else {
                // A crash may occur after the atomic apply but before the
                // marker is fsynced. Reverse-check proves that exact combined
                // patch is already present; anything else is ambiguous and
                // fails closed with every worktree retained.
                let mut reverse_check = self.integration_apply_command();
                reverse_check.args(["apply", "--reverse", "--check", "--whitespace=nowarn", "-"]);
                command_with_stdin(&mut reverse_check, &combined)
                    .context("combined patch is neither safely applicable nor already applied")?;
            }
        }
        persist_integration_marker(&marker, &self.group_id)?;
        if let Some(build_context) = build_context {
            for change in integrated_changes {
                build_context.record_modification(change.path, change.task_id);
                build_context.record_line_changes(change.additions, change.deletions);
            }
        }
        Ok(())
    }

    fn integration_apply_command(&self) -> Command {
        let mut command = Command::new("git");
        match self.backend {
            IsolationBackend::GitWorktree => {
                command.arg("-C").arg(&self.repo_root);
            }
            IsolationBackend::SnapshotRepository => {
                command
                    .arg("-C")
                    .arg(&self.repo_root)
                    .arg("--git-dir")
                    .arg(self.base_dir.join(SNAPSHOT_REPOSITORY_NAME))
                    .arg("--work-tree")
                    .arg(&self.repo_root);
            }
        }
        command
    }
}

fn stage_declared_generated_artifacts(
    workspace: &IsolatedBuildWorkspace,
    result: &SubAgentResult,
) -> Result<()> {
    let Some(handoff) = result.delegated_handoff() else {
        return Ok(());
    };
    ensure!(
        handoff.generated_artifacts.len() <= 16,
        "delegated handoff declares more than 16 generated artifacts"
    );
    for artifact in handoff.generated_artifacts {
        let relative = Path::new(artifact.path.trim());
        ensure!(
            !artifact.path.trim().is_empty()
                && !relative.is_absolute()
                && relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "declared generated artifact must be a normalized relative path: {}",
            artifact.path
        );
        ensure!(
            !relative.components().any(|component| {
                let Component::Normal(name) = component else {
                    return true;
                };
                name == ".git"
                    || GENERATED_INTEGRATION_EXCLUDES
                        .iter()
                        .any(|excluded| name == std::ffi::OsStr::new(excluded))
            }),
            "declared generated artifact is a prohibited cache or runtime path: {}",
            artifact.path
        );
        let source = workspace.root.join(relative);
        let metadata = fs::symlink_metadata(&source)
            .with_context(|| format!("inspect declared generated artifact {}", artifact.path))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "declared generated artifact may not be a symlink: {}",
            artifact.path
        );

        let mut add = Command::new("git");
        add.arg("-C")
            .arg(&workspace.root)
            .args(["add", "-f", "--"])
            .arg(relative);
        command_output(&mut add)
            .with_context(|| format!("stage declared generated artifact {}", artifact.path))?;

        let mut indexed = Command::new("git");
        indexed
            .arg("-C")
            .arg(&workspace.root)
            .args(["ls-files", "-s", "-z", "--"])
            .arg(relative);
        let indexed = command_output(&mut indexed)?.stdout;
        ensure!(
            !indexed
                .split(|byte| *byte == 0)
                .any(|entry| { entry.starts_with(b"120000 ") || entry.starts_with(b"160000 ") }),
            "declared generated artifact contains a symlink or nested repository: {}",
            artifact.path
        );
    }
    Ok(())
}

struct IntegrationChange {
    task_id: String,
    path: PathBuf,
    additions: usize,
    deletions: usize,
}

fn parse_integration_numstat(task_id: &str, bytes: &[u8]) -> Vec<IntegrationChange> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            let field = String::from_utf8_lossy(field);
            let mut parts = field.splitn(3, '\t');
            let additions = parts.next()?.parse().unwrap_or(0);
            let deletions = parts.next()?.parse().unwrap_or(0);
            let path = PathBuf::from(parts.next()?);
            Some(IntegrationChange {
                task_id: task_id.to_string(),
                path,
                additions,
                deletions,
            })
        })
        .collect()
}

fn prepare_one(repo: &Path, root: &Path, source_patch: &[u8], untracked: &[u8]) -> Result<()> {
    command_output(Command::new("git").args([
        "-C",
        &repo.display().to_string(),
        "worktree",
        "add",
        "--detach",
        &root.display().to_string(),
        "HEAD",
    ]))?;
    if !source_patch.is_empty() {
        command_with_stdin(
            Command::new("git").args([
                "-C",
                &root.display().to_string(),
                "apply",
                "--whitespace=nowarn",
                "-",
            ]),
            source_patch,
        )?;
    }
    for relative in untracked
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let relative = PathBuf::from(String::from_utf8(relative.to_vec())?);
        let source = repo.join(&relative);
        let destination = root.join(&relative);
        copy_snapshot_entry(&source, &destination)?;
    }
    command_output(Command::new("git").args(["-C", &root.display().to_string(), "add", "-A"]))?;
    command_output(Command::new("git").args([
        "-C",
        &root.display().to_string(),
        "-c",
        "user.name=Mitsuro",
        "-c",
        "user.email=runtime@mitsuro.local",
        "commit",
        "--allow-empty",
        "--no-gpg-sign",
        "-m",
        "mitsuro isolated build baseline",
    ]))?;
    Ok(())
}

fn create_snapshot_repository(authoritative: &Path, base_dir: &Path) -> Result<String> {
    let snapshot_repository = base_dir.join(SNAPSHOT_REPOSITORY_NAME);
    command_output(Command::new("git").args([
        "init",
        "--bare",
        &snapshot_repository.display().to_string(),
    ]))?;

    // Install generated-tree policy before `git add` so ignored and unignored
    // dependency/runtime files are never hashed into the temporary object
    // database. Removing them from the index after staging is still retained
    // below as a fail-closed defense, but should normally be a no-op.
    let generated_excludes = GENERATED_INTEGRATION_EXCLUDES
        .iter()
        // A slashless directory pattern applies at every depth, matching the
        // integration pathspec policy below. This prevents nested package or
        // Rust workspaces from hashing generated trees into snapshots.
        .map(|path| format!("{path}/\n"))
        .collect::<String>();
    fs::write(snapshot_repository.join("info/exclude"), generated_excludes)
        .context("install generated state exclusions for authoritative snapshot")?;

    let mut add = Command::new("git");
    add.current_dir(authoritative)
        .env("GIT_DIR", &snapshot_repository)
        .env("GIT_WORK_TREE", authoritative)
        .args(["add", "-A", "--", "."]);
    // A snapshot is source authority, not a copy of disposable build state.
    // Enforce the same cache/runtime exclusions used during integration even
    // when a model forgot to create a project-local .gitignore. Apart from
    // avoiding needless I/O, this prevents package-manager symlinks (notably
    // node_modules/.bin) from invalidating an otherwise safe source snapshot.
    command_output(&mut add).context("capture authoritative snapshot index")?;

    // Do not name generated paths in the add pathspec. Git rejects an
    // explicitly named ignored path even when a later exclusion pathspec
    // would remove it, which made a normal scaffold .gitignore turn an
    // otherwise valid dependency wave into a pre-admission failure. Add the
    // authoritative tree normally (honouring its ignore rules), then remove
    // any unignored generated state from the temporary index.
    let mut unstage_generated = Command::new("git");
    unstage_generated
        .current_dir(authoritative)
        .env("GIT_DIR", &snapshot_repository)
        .env("GIT_WORK_TREE", authoritative)
        .args(["rm", "-r", "--cached", "--ignore-unmatch", "--"]);
    add_generated_pathspecs(&mut unstage_generated);
    command_output(&mut unstage_generated)
        .context("exclude generated state from authoritative snapshot index")?;

    let mut listed = Command::new("git");
    listed
        .current_dir(authoritative)
        .env("GIT_DIR", &snapshot_repository)
        .env("GIT_WORK_TREE", authoritative)
        .args(["ls-files", "-s", "-z"]);
    let indexed = command_output(&mut listed)?.stdout;
    ensure!(
        !indexed
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"120000 ")),
        "symlinks are not supported in unborn or non-Git isolated snapshots"
    );

    let mut write_tree = Command::new("git");
    write_tree
        .current_dir(authoritative)
        .env("GIT_DIR", &snapshot_repository)
        .env("GIT_WORK_TREE", authoritative)
        .arg("write-tree");
    let tree = String::from_utf8(command_output(&mut write_tree)?.stdout)?
        .trim()
        .to_string();
    ensure!(!tree.is_empty(), "snapshot tree was not created");

    let mut commit_tree = Command::new("git");
    commit_tree
        .env("GIT_DIR", &snapshot_repository)
        .env("GIT_AUTHOR_NAME", "Mitsuro")
        .env("GIT_AUTHOR_EMAIL", "runtime@mitsuro.local")
        .env("GIT_COMMITTER_NAME", "Mitsuro")
        .env("GIT_COMMITTER_EMAIL", "runtime@mitsuro.local")
        .args(["commit-tree", &tree]);
    let commit = String::from_utf8(
        command_with_stdin(&mut commit_tree, b"mitsuro isolated snapshot baseline\n")?.stdout,
    )?
    .trim()
    .to_string();
    ensure!(!commit.is_empty(), "snapshot commit was not created");
    command_output(Command::new("git").args([
        "--git-dir",
        &snapshot_repository.display().to_string(),
        "update-ref",
        "refs/heads/snapshot",
        &commit,
    ]))?;
    Ok(commit)
}

fn prepare_snapshot_worktree(snapshot_repository: &Path, root: &Path, commit: &str) -> Result<()> {
    command_output(Command::new("git").args([
        "--git-dir",
        &snapshot_repository.display().to_string(),
        "worktree",
        "add",
        "--detach",
        &root.display().to_string(),
        commit,
    ]))?;
    Ok(())
}

fn add_generated_pathspecs(command: &mut Command) {
    for excluded in GENERATED_INTEGRATION_EXCLUDES {
        command
            .arg(excluded)
            .arg(format!(":(glob)**/{excluded}"))
            .arg(format!(":(glob)**/{excluded}/**"));
    }
}

fn add_integration_excluded_pathspecs(command: &mut Command) {
    add_generated_pathspecs(command);
    for excluded in RESTAGEABLE_BUILD_OUTPUT_DIRS {
        command
            .arg(excluded)
            .arg(format!(":(glob)**/{excluded}"))
            .arg(format!(":(glob)**/{excluded}/**"));
    }
    for pattern in RESTAGEABLE_BUILD_OUTPUT_GLOBS {
        command.arg(format!(":(glob)**/{pattern}"));
    }
}

#[cfg(unix)]
fn link_authoritative_node_modules(
    repo: &Path,
    project_dir: &Path,
    working_dir: &Path,
    isolated_root: &Path,
) -> Result<()> {
    use std::os::unix::fs::symlink;

    let mut candidates = std::collections::BTreeSet::new();
    for start in [project_dir, working_dir] {
        let mut current = Some(start);
        while let Some(directory) = current {
            if !directory.starts_with(repo) {
                break;
            }
            candidates.insert(directory.to_path_buf());
            if directory == repo {
                break;
            }
            current = directory.parent();
        }
    }

    for directory in candidates {
        let source = directory.join("node_modules");
        let Ok(metadata) = fs::symlink_metadata(&source) else {
            continue;
        };
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "authoritative dependency root is not a regular directory: {}",
            source.display()
        );
        let relative = directory
            .strip_prefix(repo)
            .context("resolve authoritative dependency root")?;
        let destination = isolated_root.join(relative).join("node_modules");
        if destination.exists() {
            continue;
        }
        ensure!(
            fs::symlink_metadata(&destination).is_err(),
            "isolated dependency path is an invalid dangling entry: {}",
            destination.display()
        );
        fs::create_dir(&destination).with_context(|| {
            format!(
                "create isolated dependency facade {}",
                destination.display()
            )
        })?;
        let mut entries = fs::read_dir(&source)
            .with_context(|| format!("read authoritative dependencies {}", source.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            if WRITABLE_NODE_MODULES_ENTRIES
                .iter()
                .any(|writable| name == std::ffi::OsStr::new(writable))
            {
                continue;
            }
            let target = entry.path();
            let link = destination.join(&name);
            symlink(&target, &link).with_context(|| {
                format!(
                    "link authoritative dependency {} into isolated facade",
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn link_authoritative_node_modules(
    _repo: &Path,
    _project_dir: &Path,
    _working_dir: &Path,
    _isolated_root: &Path,
) -> Result<()> {
    Ok(())
}

fn raw_baseline_commit(baseline: &str) -> Result<&str> {
    let baseline = baseline
        .strip_prefix(SNAPSHOT_BASELINE_PREFIX)
        .unwrap_or(baseline)
        .trim();
    ensure!(
        !baseline.is_empty(),
        "durable isolation baseline is missing"
    );
    Ok(baseline)
}

fn copy_snapshot_entry(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("inspect snapshot entry {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "untracked symlink is not supported in isolated build snapshots: {}",
            source.display()
        );
    }
    ensure!(
        metadata.is_file(),
        "untracked snapshot entry is not a regular file: {}",
        source.display()
    );
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "copy snapshot entry {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn path_is_ignored_by_repository(repo: &Path, project_dir: &Path) -> Result<bool> {
    if project_dir == repo {
        return Ok(false);
    }
    let relative = project_dir
        .strip_prefix(repo)
        .context("resolve project path for Git ignore policy")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["check-ignore", "-q", "--"])
        .arg(relative)
        .output()
        .context("inspect enclosing repository ignore policy")?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            bail!(
                "git check-ignore failed for {}: {detail}",
                project_dir.display()
            )
        }
    }
}

fn validate_group_id(group_id: &str) -> Result<()> {
    ensure!(
        !group_id.is_empty()
            && group_id.len() <= 128
            && group_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-'),
        "invalid delegation group id for isolated workspace"
    );
    Ok(())
}

fn materialization_lock_slot(group_id: &str) -> u8 {
    // Stable FNV-1a keeps the lock namespace bounded without introducing a
    // process-randomized hasher or an unbounded file per delegation run.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in group_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash % 64) as u8
}

fn isolation_root() -> Result<PathBuf> {
    #[cfg(unix)]
    let suffix = unsafe { libc::geteuid() }.to_string();
    #[cfg(not(unix))]
    let suffix = "user".to_string();
    let root = std::env::temp_dir().join(format!("mitsuro-delegation-{suffix}"));
    if root.exists() {
        let metadata = fs::symlink_metadata(&root)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid isolation root"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                "isolation root is owned by a different user"
            );
        }
    } else {
        fs::create_dir(&root)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    Ok(root)
}

fn resolve_repo(path: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        String::from_utf8(
            command_output(Command::new("git").args([
                "-C",
                &path.display().to_string(),
                "rev-parse",
                "--show-toplevel",
            ]))?
            .stdout,
        )?
        .trim(),
    ))
}

fn git_common_dir(repo: &Path) -> Result<PathBuf> {
    let raw = PathBuf::from(
        String::from_utf8(
            command_output(Command::new("git").args([
                "-C",
                &repo.display().to_string(),
                "rev-parse",
                "--git-common-dir",
            ]))?
            .stdout,
        )?
        .trim(),
    );
    Ok(if raw.is_absolute() {
        raw
    } else {
        repo.join(raw)
    })
}

fn integration_marker_for(
    repo: &Path,
    group_id: &str,
    backend: IsolationBackend,
) -> Result<PathBuf> {
    validate_group_id(group_id)?;
    match backend {
        IsolationBackend::GitWorktree => Ok(git_common_dir(repo)?
            .join("mitsuro-delegation-integrated")
            .join(group_id)),
        IsolationBackend::SnapshotRepository => {
            Ok(isolation_root()?.join("integrated").join(group_id))
        }
    }
}

fn persist_integration_marker(path: &Path, group_id: &str) -> Result<()> {
    let parent = path.parent().context("integration marker parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{group_id}.tmp-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(group_id.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn integration_marker_is_valid(path: &Path, group_id: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "delegation integration marker is not an owned regular file"
    );
    ensure!(
        metadata.len() <= 128,
        "delegation integration marker exceeds its bounded size"
    );
    ensure!(
        fs::read_to_string(path)? == group_id,
        "delegation integration marker does not match its durable group"
    );
    Ok(true)
}

fn remove_worktree(repo: &Path, root: &Path) -> Result<()> {
    command_output(Command::new("git").args([
        "-C",
        &repo.display().to_string(),
        "worktree",
        "remove",
        "--force",
        &root.display().to_string(),
    ]))?;
    Ok(())
}

fn rewrite_result_paths(result: &mut SubAgentResult, isolated: &Path, authoritative: &Path) {
    for path in &mut result.files_examined {
        let candidate = PathBuf::from(path.as_str());
        if let Ok(relative) = candidate.strip_prefix(isolated) {
            *path = authoritative.join(relative).display().to_string();
        }
    }
}

fn command_output(command: &mut Command) -> Result<Output> {
    let rendered = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("execute {rendered}"))?;
    if output.status.success() {
        return Ok(output);
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    bail!("{rendered} failed: {detail}")
}

fn command_with_stdin(command: &mut Command, input: &[u8]) -> Result<Output> {
    let rendered = format!("{command:?}");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("execute {rendered}"))?;
    child
        .stdin
        .take()
        .context("git command stdin unavailable")?
        .write_all(input)?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        return Ok(output);
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    bail!("{rendered} failed: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn result(task_id: &str) -> SubAgentResult {
        SubAgentResult {
            task_id: task_id.to_string(),
            agent_name: task_id.to_string(),
            delegated_run_id: Some("group-test".to_string()),
            success: true,
            output: "done".to_string(),
            files_examined: Vec::new(),
            duration_ms: 1,
            turns_used: 1,
            error: None,
            termination: SubAgentTermination::Completed,
            policy_violations: Vec::new(),
            evidence: Default::default(),
            background_processes: Vec::new(),
        }
    }

    fn failed_result(task_id: &str) -> SubAgentResult {
        let mut result = result(task_id);
        result.success = false;
        result.termination = SubAgentTermination::Failed;
        result.error = Some("unfinished before restart".to_string());
        result
    }

    fn degraded_mutation_result(task_id: &str) -> SubAgentResult {
        let mut result = result(task_id);
        result.success = false;
        result.termination = SubAgentTermination::ProviderTimeout;
        result.error = Some("provider timed out after producing a partial patch".to_string());
        result.evidence.record_attempt();
        result
            .evidence
            .record_success(super::super::DelegatedEvidenceKind::Mutation);
        result
    }

    fn commit_base(repo: &Path) {
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
    }

    #[tokio::test]
    async fn ignored_nested_project_uses_project_rooted_snapshot_and_integrates() {
        let temp = tempfile::TempDir::new().expect("temp repository");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "/artifacts/\n").expect("ignore artifacts");
        fs::write(repo.join("README.md"), "parent repository\n").expect("parent readme");
        commit_base(repo);

        let project = repo.join("artifacts/game");
        fs::create_dir_all(&project).expect("ignored project directory");
        fs::write(project.join("README.md"), "game project\n").expect("project readme");
        fs::write(project.join(".gitignore"), "node_modules\n").expect("project dependency ignore");
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            project.clone(),
            project.clone(),
            group_id,
            vec!["scaffold".to_string()],
        )
        .await
        .expect("prepare ignored project isolation")
        .expect("snapshot isolation");

        let workspace = &isolation.workspaces()[0];
        assert!(workspace
            .baseline_commit
            .starts_with(SNAPSHOT_BASELINE_PREFIX));
        assert_eq!(workspace.project_dir, workspace.root);
        assert_eq!(workspace.working_dir, workspace.root);
        assert_eq!(
            fs::read_to_string(workspace.root.join("README.md")).unwrap(),
            "game project\n"
        );
        assert!(
            !workspace.root.join("../README.md").exists(),
            "snapshot must be rooted at the selected project, not its enclosing repository"
        );
        fs::write(workspace.root.join("package.json"), "{}\n").expect("scaffold output");
        let mut task_result = result("scaffold");
        task_result.evidence.record_attempt();
        task_result
            .evidence
            .record_success(super::super::DelegatedEvidenceKind::Mutation);

        let integrated = isolation.integrate(vec![task_result]).await;
        assert!(integrated[0].success, "{integrated:?}");
        assert_eq!(
            fs::read_to_string(project.join("package.json")).unwrap(),
            "{}\n"
        );
        assert!(
            !repo.join("package.json").exists(),
            "nested project output must not escape into the enclosing repository root"
        );
    }

    #[tokio::test]
    async fn snapshot_excludes_generated_dependency_and_runtime_trees() {
        let temp = tempfile::TempDir::new().expect("temp repository");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "/artifacts/\n").expect("ignore artifacts");
        fs::write(repo.join("README.md"), "parent repository\n").expect("parent readme");
        commit_base(repo);

        let project = repo.join("artifacts/game");
        fs::create_dir_all(project.join("node_modules/pkg")).expect("dependency directory");
        fs::create_dir_all(project.join("node_modules/.bin")).expect("binary directory");
        fs::create_dir_all(project.join(".mitsuro/cache")).expect("runtime directory");
        fs::create_dir_all(project.join("dist")).expect("distribution directory");
        fs::write(project.join("README.md"), "game project\n").expect("project readme");
        fs::write(project.join("node_modules/pkg/index.js"), "dependency\n")
            .expect("dependency file");
        #[cfg(unix)]
        std::os::unix::fs::symlink("../pkg/index.js", project.join("node_modules/.bin/pkg"))
            .expect("package-manager symlink");
        fs::write(project.join(".mitsuro/cache/state"), "runtime\n").expect("runtime file");
        fs::write(project.join("dist/app.js"), "product artifact\n").expect("dist file");

        let isolation = BuildIsolationSet::prepare(
            project.clone(),
            project.clone(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["engine".to_string()],
        )
        .await
        .expect("generated trees must not invalidate snapshot isolation")
        .expect("snapshot isolation");

        let dependency_oid = String::from_utf8(
            command_output(
                Command::new("git")
                    .current_dir(&project)
                    .args(["hash-object", "node_modules/pkg/index.js"]),
            )
            .expect("hash dependency fixture")
            .stdout,
        )
        .expect("dependency object id");
        let snapshot_repository = isolation.workspaces()[0]
            .root
            .parent()
            .expect("wave directory")
            .join(SNAPSHOT_REPOSITORY_NAME);
        let stored_dependency = Command::new("git")
            .args([
                "--git-dir",
                &snapshot_repository.display().to_string(),
                "cat-file",
                "-e",
                dependency_oid.trim(),
            ])
            .output()
            .expect("inspect snapshot object database");
        assert!(
            !stored_dependency.status.success(),
            "excluded dependency content must never enter the snapshot object database"
        );

        for workspace in isolation.workspaces() {
            assert!(workspace.root.join("README.md").is_file());
            assert!(workspace.root.join("dist/app.js").is_file());
            let linked_dependencies = workspace.root.join("node_modules");
            assert!(
                linked_dependencies.is_dir()
                    && !fs::symlink_metadata(&linked_dependencies)
                        .expect("dependency facade")
                        .file_type()
                        .is_symlink(),
                "dependency facade itself must remain writable"
            );
            assert_eq!(
                linked_dependencies.join("pkg").canonicalize().unwrap(),
                project.join("node_modules/pkg").canonicalize().unwrap()
            );
            fs::create_dir_all(linked_dependencies.join(".vite-temp"))
                .expect("write isolated Vite cache");
            fs::write(
                linked_dependencies.join(".vite-temp/config.mjs"),
                "isolated cache\n",
            )
            .expect("write isolated cache file");
            assert!(
                !project.join("node_modules/.vite-temp/config.mjs").exists(),
                "isolated tool caches must not mutate authoritative dependencies"
            );
            assert_eq!(
                fs::read_to_string(linked_dependencies.join("pkg/index.js")).unwrap(),
                "dependency\n"
            );
            assert!(
                !workspace.root.join(".mitsuro/cache/state").exists(),
                "authoritative runtime state must not enter the snapshot"
            );
        }

        fs::create_dir_all(isolation.workspaces()[0].root.join("src"))
            .expect("isolated source directory");
        fs::write(
            isolation.workspaces()[0].root.join("src/engine.js"),
            "export const ready = true;\n",
        )
        .expect("isolated source output");
        let mut engine = result("engine");
        engine.evidence.record_attempt();
        engine
            .evidence
            .record_success(super::super::DelegatedEvidenceKind::Mutation);
        let integrated = isolation.integrate(vec![engine]).await;
        assert!(integrated[0].success, "{integrated:?}");
        assert!(project.join("src/engine.js").is_file());
        assert!(
            fs::symlink_metadata(project.join("node_modules"))
                .expect("authoritative dependencies")
                .is_dir(),
            "dependency link must never replace the authoritative dependency directory"
        );
    }

    #[tokio::test]
    async fn degraded_partial_mutation_is_integrated_before_dependency_release() {
        let temp = tempfile::TempDir::new().expect("temp repository");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("README.md"), "base\n").expect("base file");
        commit_base(repo);
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["partial".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        fs::write(
            isolation.workspaces()[0].root.join("partial.txt"),
            "usable partial output\n",
        )
        .expect("partial output");

        let integrated = isolation
            .integrate(vec![degraded_mutation_result("partial")])
            .await;
        assert!(!integrated[0].success, "degraded status remains truthful");
        assert_eq!(
            integrated[0].termination,
            SubAgentTermination::ProviderTimeout
        );
        assert_eq!(
            fs::read_to_string(repo.join("partial.txt")).unwrap(),
            "usable partial output\n"
        );
    }

    #[tokio::test]
    async fn reported_mutation_with_empty_patch_fails_closed() {
        let temp = tempfile::TempDir::new().expect("temp repository");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("README.md"), "base\n").expect("base file");
        commit_base(repo);
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["missing".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let recovery_root = isolation.workspaces()[0].root.clone();
        let mut task_result = result("missing");
        task_result.evidence.record_attempt();
        task_result
            .evidence
            .record_success(super::super::DelegatedEvidenceKind::Mutation);

        let integrated = isolation.integrate(vec![task_result]).await;
        assert!(!integrated[0].success);
        assert_eq!(integrated[0].termination, SubAgentTermination::Failed);
        assert!(integrated[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("integration patch was empty")));
        assert!(
            recovery_root.exists(),
            "ambiguous workspace must be retained"
        );
        remove_worktree(repo, &recovery_root).expect("remove retained test worktree");
        fs::remove_dir(recovery_root.parent().expect("batch root"))
            .expect("remove retained test batch");
    }

    #[tokio::test]
    async fn dirty_snapshot_isolated_patches_integrate_in_task_order() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::create_dir_all(repo.join("src")).expect("src directory");
        fs::write(repo.join("src/base.txt"), "committed\n").expect("base file");
        fs::write(repo.join(".gitignore"), "target/\n").expect("base ignore file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        fs::write(repo.join("src/base.txt"), "dirty source\n").expect("dirty source");
        fs::write(repo.join("src/untracked.txt"), "untracked source\n").expect("untracked source");

        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id,
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        assert_eq!(
            fs::read_to_string(isolation.workspaces()[0].root.join("src/base.txt"))
                .expect("isolated dirty file"),
            "dirty source\n"
        );
        assert_eq!(
            fs::read_to_string(isolation.workspaces()[1].root.join("src/untracked.txt"))
                .expect("isolated untracked file"),
            "untracked source\n"
        );
        fs::write(isolation.workspaces()[0].root.join("src/a.txt"), "a\n").expect("task a output");
        fs::write(isolation.workspaces()[1].root.join("src/b.txt"), "b\n").expect("task b output");
        fs::create_dir_all(isolation.workspaces()[0].root.join("target/debug"))
            .expect("generated target directory");
        fs::write(
            isolation.workspaces()[0]
                .root
                .join("target/debug/cache.bin"),
            vec![7_u8; 1024],
        )
        .expect("generated target artifact");
        fs::create_dir_all(isolation.workspaces()[1].root.join(".cargo-home/registry"))
            .expect("generated cargo home");
        fs::write(
            isolation.workspaces()[1]
                .root
                .join(".cargo-home/registry/cache.bin"),
            vec![9_u8; 1024],
        )
        .expect("generated cargo cache");
        assert!(!repo.join("src/a.txt").exists());

        let context = Arc::new(SharedBuildContext::new());
        let results = isolation
            .integrate_recording(vec![result("task-a"), result("task-b")], context.clone())
            .await;
        assert!(results.iter().all(|result| result.success), "{results:?}");
        assert_eq!(fs::read_to_string(repo.join("src/a.txt")).unwrap(), "a\n");
        assert_eq!(fs::read_to_string(repo.join("src/b.txt")).unwrap(), "b\n");
        assert!(
            !repo.join("target").exists(),
            "delegated build caches must never enter the authoritative patch"
        );
        assert!(
            !repo.join(".cargo-home").exists(),
            "delegated package caches must never enter the authoritative patch"
        );
        assert_eq!(
            fs::read_to_string(repo.join("src/base.txt")).unwrap(),
            "dirty source\n"
        );
        let stats = context.stats();
        assert_eq!(stats.files_modified, 2);
        assert_eq!(stats.lines_added, 2);
        assert_eq!(stats.lines_removed, 0);
    }

    #[tokio::test]
    async fn parallel_validation_outputs_do_not_conflict_disjoint_source_patches() {
        let temp = tempfile::TempDir::new().expect("temporary project");
        let project = temp.path();
        fs::create_dir_all(project.join("src")).expect("source directory");
        fs::create_dir_all(project.join("dist")).expect("distribution directory");
        fs::write(project.join("src/main.ts"), "export const ready = true;\n")
            .expect("base source");
        fs::write(project.join("dist/index.html"), "baseline dist\n").expect("base distribution");
        fs::write(project.join("tsconfig.tsbuildinfo"), "baseline metadata\n")
            .expect("base compiler metadata");

        let isolation = BuildIsolationSet::prepare(
            project.to_path_buf(),
            project.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["engine".to_string(), "interface".to_string()],
        )
        .await
        .expect("prepare snapshot isolation")
        .expect("snapshot isolation");

        let engine = &isolation.workspaces()[0].root;
        fs::write(
            engine.join("src/engine.ts"),
            "export const engine = true;\n",
        )
        .expect("engine source");
        fs::write(engine.join("dist/index.html"), "engine build\n").expect("engine distribution");
        fs::write(engine.join("tsconfig.tsbuildinfo"), "engine metadata\n")
            .expect("engine compiler metadata");

        let interface = &isolation.workspaces()[1].root;
        fs::write(
            interface.join("src/interface.ts"),
            "export const ui = true;\n",
        )
        .expect("interface source");
        fs::write(interface.join("dist/index.html"), "interface build\n")
            .expect("interface distribution");
        fs::write(
            interface.join("tsconfig.tsbuildinfo"),
            "interface metadata\n",
        )
        .expect("interface compiler metadata");

        let integrated = isolation
            .integrate(vec![result("engine"), result("interface")])
            .await;
        assert!(
            integrated.iter().all(|result| result.success),
            "{integrated:?}"
        );
        assert!(project.join("src/engine.ts").is_file());
        assert!(project.join("src/interface.ts").is_file());
        assert_eq!(
            fs::read_to_string(project.join("dist/index.html")).unwrap(),
            "baseline dist\n",
            "parallel validation builds must not publish generated output implicitly"
        );
        assert_eq!(
            fs::read_to_string(project.join("tsconfig.tsbuildinfo")).unwrap(),
            "baseline metadata\n",
            "parallel compiler metadata must not enter source patches"
        );
    }

    #[tokio::test]
    async fn explicitly_declared_ignored_deliverable_crosses_isolation_boundary() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "dist/\ntarget/\n").expect("ignore file");
        fs::write(repo.join("package.json"), "{}\n").expect("package file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );

        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["task-a".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let workspace = &isolation.workspaces()[0];
        fs::create_dir_all(workspace.root.join("dist/assets")).expect("dist directory");
        fs::write(workspace.root.join("dist/index.html"), "ready\n").expect("dist index");
        fs::write(workspace.root.join("dist/assets/app.js"), "ok\n").expect("dist asset");
        fs::create_dir_all(workspace.root.join("target/debug")).expect("target directory");
        fs::write(workspace.root.join("target/debug/cache"), "cache\n").expect("target cache");

        let mut task_result = result("task-a");
        task_result.output = r#"Build and validation completed.
<delegated_handoff>{"status":"complete","summary":"built distributable","acceptance_checks":[{"id":"build","status":"passed","evidence":"bundle emitted"}],"remaining_work":[],"blockers":[],"generated_artifacts":[{"path":"dist","purpose":"final web distributable"}]}</delegated_handoff>"#.to_string();
        let integrated = isolation.integrate(vec![task_result]).await;

        assert!(integrated[0].success, "{integrated:?}");
        assert_eq!(
            fs::read_to_string(repo.join("dist/index.html")).unwrap(),
            "ready\n"
        );
        assert_eq!(
            fs::read_to_string(repo.join("dist/assets/app.js")).unwrap(),
            "ok\n"
        );
        assert!(!repo.join("target").exists());
    }

    #[tokio::test]
    async fn declared_cache_artifact_fails_closed_and_retains_workspace() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "target/\n").expect("ignore file");
        fs::write(repo.join("README.md"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );

        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["task-a".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let recovery_root = isolation.workspaces()[0].root.clone();
        fs::create_dir_all(recovery_root.join("target/debug")).expect("target directory");
        fs::write(recovery_root.join("target/debug/cache"), "cache\n").expect("target cache");
        let mut task_result = result("task-a");
        task_result.output = r#"<delegated_handoff>{"status":"complete","summary":"done","acceptance_checks":[],"remaining_work":[],"blockers":[],"generated_artifacts":[{"path":"target","purpose":"cache"}]}</delegated_handoff>"#.to_string();

        let integrated = isolation.integrate(vec![task_result]).await;

        assert!(!integrated[0].success);
        assert!(integrated[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("prohibited cache")));
        assert!(recovery_root.exists());
        assert!(!repo.join("target").exists());
    }

    #[tokio::test]
    async fn unborn_git_workspace_uses_private_snapshot_without_creating_head() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::create_dir_all(repo.join("src")).expect("src directory");
        fs::write(repo.join("src/main.js"), "export const ready = false;\n")
            .expect("unborn source");

        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id,
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare unborn isolation")
        .expect("unborn isolation");
        assert_eq!(isolation.backend, IsolationBackend::SnapshotRepository);
        assert!(isolation.workspaces().iter().all(|workspace| workspace
            .baseline_commit
            .starts_with(SNAPSHOT_BASELINE_PREFIX)));
        fs::write(
            isolation.workspaces()[0].root.join("src/a.js"),
            "export const a = 1;\n",
        )
        .expect("task a output");
        fs::write(
            isolation.workspaces()[1].root.join("src/b.js"),
            "export const b = 2;\n",
        )
        .expect("task b output");

        let results = isolation
            .integrate(vec![result("task-a"), result("task-b")])
            .await;
        assert!(results.iter().all(|result| result.success), "{results:?}");
        assert!(repo.join("src/a.js").exists());
        assert!(repo.join("src/b.js").exists());
        assert!(
            !Command::new("git")
                .args(["rev-parse", "--verify", "HEAD"])
                .current_dir(repo)
                .output()
                .expect("inspect unborn head")
                .status
                .success(),
            "snapshot isolation must not create the authoritative first commit"
        );
    }

    #[tokio::test]
    async fn non_git_workspace_isolated_snapshot_integrates_and_restores_without_git_metadata() {
        // The repository pins TMPDIR under target/ for Rust builds. A default
        // TempDir would therefore still be inside this Git worktree and would
        // not exercise the non-Git backend at all.
        let temp_root = if cfg!(windows) {
            std::env::temp_dir()
        } else {
            PathBuf::from("/tmp")
        };
        let temp = tempfile::Builder::new()
            .prefix("mitsuro-non-git-")
            .tempdir_in(temp_root)
            .expect("temp project outside repository");
        let project = temp.path();
        fs::create_dir_all(project.join("src")).expect("src directory");
        fs::write(project.join("src/main.txt"), "source\n").expect("source file");
        fs::write(project.join(".gitignore"), "ignored.txt\n").expect("ignore file");
        fs::write(project.join("ignored.txt"), "do not snapshot\n").expect("ignored file");

        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            project.to_path_buf(),
            project.to_path_buf(),
            group_id.clone(),
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare non-git isolation")
        .expect("non-git isolation");
        assert_eq!(isolation.backend, IsolationBackend::SnapshotRepository);
        assert!(!isolation.workspaces()[0].root.join("ignored.txt").exists());
        let durable = isolation
            .workspaces()
            .iter()
            .map(|workspace| {
                (
                    workspace.task_id.clone(),
                    workspace.root.clone(),
                    workspace.baseline_commit.clone(),
                )
            })
            .collect::<Vec<_>>();
        fs::write(isolation.workspaces()[0].root.join("src/a.txt"), "a\n").expect("task a output");
        fs::write(isolation.workspaces()[1].root.join("src/b.txt"), "b\n").expect("task b output");
        drop(isolation);

        let restored = BuildIsolationSet::restore(project.to_path_buf(), group_id, durable)
            .await
            .expect("restore non-git isolation");
        let results = restored
            .integrate(vec![result("task-a"), result("task-b")])
            .await;
        assert!(results.iter().all(|result| result.success), "{results:?}");
        assert_eq!(
            fs::read_to_string(project.join("src/a.txt")).unwrap(),
            "a\n"
        );
        assert_eq!(
            fs::read_to_string(project.join("src/b.txt")).unwrap(),
            "b\n"
        );
        assert!(!project.join(".git").exists());
    }

    #[tokio::test]
    async fn successful_no_op_builds_integrate_without_a_synthetic_patch() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "base",
            ],
        );

        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            format!("group-{}", uuid::Uuid::new_v4()),
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let roots = isolation
            .workspaces()
            .iter()
            .map(|workspace| workspace.root.clone())
            .collect::<Vec<_>>();

        let context = Arc::new(SharedBuildContext::new());
        let results = isolation
            .integrate_recording(vec![result("task-a"), result("task-b")], context.clone())
            .await;

        assert!(results.iter().all(|result| result.success), "{results:?}");
        assert!(roots.iter().all(|root| !root.exists()));
        assert_eq!(context.stats().files_modified, 0);
    }

    #[tokio::test]
    async fn conflicting_batch_is_all_or_nothing_and_retains_recovery_worktrees() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("shared.txt"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id,
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let roots = isolation
            .workspaces()
            .iter()
            .map(|workspace| workspace.root.clone())
            .collect::<Vec<_>>();
        fs::write(roots[0].join("shared.txt"), "a\n").expect("task a");
        fs::write(roots[1].join("shared.txt"), "b\n").expect("task b");

        let results = isolation
            .integrate(vec![result("task-a"), result("task-b")])
            .await;

        assert!(results.iter().all(|result| !result.success));
        assert_eq!(
            fs::read_to_string(repo.join("shared.txt")).unwrap(),
            "base\n"
        );
        assert!(roots.iter().all(|root| root.is_dir()));
        for root in &roots {
            remove_worktree(repo, root).expect("remove retained test worktree");
        }
        fs::remove_dir(roots[0].parent().expect("batch root")).expect("remove retained test batch");
    }

    #[tokio::test]
    async fn durable_restore_rejects_database_path_escape() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("base.txt"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id.clone(),
            vec!["task-a".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let baseline = isolation.workspaces()[0].baseline_commit.clone();
        let escaped = BuildIsolationSet::restore(
            repo.to_path_buf(),
            group_id,
            vec![("task-a".to_string(), repo.to_path_buf(), baseline)],
        )
        .await;
        assert!(escaped.is_err());
        isolation.discard().await.expect("discard isolation");
    }

    #[tokio::test]
    async fn recovered_integration_applies_only_terminal_success_and_is_idempotent() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("base.txt"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id.clone(),
            vec!["terminal".to_string(), "unfinished".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let durable = isolation
            .workspaces()
            .iter()
            .map(|workspace| {
                (
                    workspace.task_id.clone(),
                    workspace.root.clone(),
                    workspace.baseline_commit.clone(),
                )
            })
            .collect::<Vec<_>>();
        let terminal_root = durable[0].1.clone();
        let unfinished_root = durable[1].1.clone();
        fs::write(terminal_root.join("terminal.txt"), "terminal\n").expect("terminal task output");
        fs::write(unfinished_root.join("partial.txt"), "unsafe partial\n")
            .expect("unfinished partial output");
        drop(isolation);

        let restored =
            BuildIsolationSet::restore(repo.to_path_buf(), group_id.clone(), durable.clone())
                .await
                .expect("restore durable isolation");
        let integrated = restored
            .integrate_recovered(
                vec![result("terminal"), failed_result("unfinished")],
                std::sync::Arc::new(|| Ok(())),
            )
            .await;
        assert!(integrated
            .iter()
            .find(|result| result.task_id == "terminal")
            .is_some_and(|result| result.success));
        assert_eq!(
            fs::read_to_string(repo.join("terminal.txt")).expect("integrated terminal patch"),
            "terminal\n"
        );
        assert!(
            !repo.join("partial.txt").exists(),
            "unfinished worktree edits must never reach the authoritative workspace"
        );
        assert!(!terminal_root.exists());
        assert!(unfinished_root.is_dir(), "unsafe worktree must be retained");
        assert_eq!(
            fs::read_to_string(unfinished_root.join("partial.txt"))
                .expect("retained unfinished output"),
            "unsafe partial\n",
            "recovery must not reset or replay atop an expired writer's partial edits"
        );

        let restored_again = BuildIsolationSet::restore(repo.to_path_buf(), group_id, durable)
            .await
            .expect("restore after integration marker");
        let integrated_again = restored_again
            .integrate_recovered(
                vec![result("terminal"), failed_result("unfinished")],
                std::sync::Arc::new(|| Ok(())),
            )
            .await;
        assert!(integrated_again
            .iter()
            .find(|result| result.task_id == "terminal")
            .is_some_and(|result| result.success));
        assert_eq!(
            fs::read_to_string(repo.join("terminal.txt")).expect("idempotent terminal patch"),
            "terminal\n"
        );
        assert!(unfinished_root.is_dir());
        assert_eq!(
            fs::read_to_string(unfinished_root.join("partial.txt"))
                .expect("idempotently retained unfinished output"),
            "unsafe partial\n"
        );

        remove_worktree(repo, &unfinished_root).expect("remove retained unfinished worktree");
        fs::remove_dir(unfinished_root.parent().expect("batch root"))
            .expect("remove retained batch root");
    }

    #[tokio::test]
    async fn recovered_integration_requires_owner_fence_before_applying_patch() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("base.txt"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let isolation = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id,
            vec!["terminal".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let root = isolation.workspaces()[0].root.clone();
        fs::write(root.join("must-not-apply.txt"), "unsafe\n").expect("task output");
        let fence_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_calls = fence_calls.clone();
        let results = isolation
            .integrate_recovered(
                vec![result("terminal")],
                std::sync::Arc::new(move || {
                    let call = observed_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if call == 0 {
                        Ok(())
                    } else {
                        anyhow::bail!("owner lost at publication boundary")
                    }
                }),
            )
            .await;
        assert_eq!(fence_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(results.iter().all(|result| !result.success));
        assert!(!repo.join("must-not-apply.txt").exists());
        assert!(root.is_dir(), "lost-owner worktree must be retained");
        remove_worktree(repo, &root).expect("remove retained test worktree");
        fs::remove_dir(root.parent().expect("batch root")).expect("remove retained test batch");
    }

    #[tokio::test]
    async fn locked_materialization_reclaims_only_abandoned_owned_worktrees() {
        let temp = tempfile::TempDir::new().expect("temp repo");
        let repo = temp.path();
        run(repo, &["init", "-q"]);
        fs::write(repo.join("base.txt"), "base\n").expect("base file");
        run(repo, &["add", "-A"]);
        run(
            repo,
            &[
                "-c",
                "user.name=Mitsuro Test",
                "-c",
                "user.email=test@mitsuro.local",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let group_id = format!("group-{}", uuid::Uuid::new_v4());
        let first_guard = BuildIsolationMaterializationGuard::acquire(group_id.clone())
            .await
            .expect("first guard");
        let abandoned = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id.clone(),
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare isolation")
        .expect("git isolation");
        let roots = abandoned
            .workspaces()
            .iter()
            .map(|workspace| workspace.root.clone())
            .collect::<Vec<_>>();
        drop(abandoned);
        drop(first_guard);

        let recovery_guard = BuildIsolationMaterializationGuard::acquire(group_id.clone())
            .await
            .expect("recovery guard");
        assert!(recovery_guard
            .remove_abandoned_preparation(repo.to_path_buf(), 2)
            .await
            .expect("recover abandoned preparation"));
        assert!(roots.iter().all(|root| !root.exists()));

        let replacement = BuildIsolationSet::prepare(
            repo.to_path_buf(),
            repo.to_path_buf(),
            group_id,
            vec!["task-a".to_string(), "task-b".to_string()],
        )
        .await
        .expect("prepare replacement")
        .expect("git isolation");
        replacement.discard().await.expect("discard replacement");
    }
}
