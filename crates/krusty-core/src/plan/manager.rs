//! Plan manager
//!
//! Manages plans with SQLite-backed storage:
//! - Strict 1:1 session-plan relationship
//! - Automatic plan deletion on session delete (CASCADE)
//! - CRUD operations for plans
//! - Backward-compatible file operations for migration

use anyhow::Result;
use chrono::Utc;
use std::path::PathBuf;

use super::file::{PlanFile, PlanStatus};
use super::lifecycle::{is_active_plan, PlanLifecycleState};
use crate::paths;
use crate::storage::{Database, PlanStore, SharedDatabase, WorkMode};

/// Manages plans with SQLite storage
pub struct PlanManager {
    /// Directory where legacy plan files are stored (for migration)
    plans_dir: PathBuf,
    /// Shared database connection for plan storage
    db: SharedDatabase,
}

impl PlanManager {
    /// Create a new plan manager with shared database
    pub fn with_shared_db(db: SharedDatabase) -> Result<Self> {
        let plans_dir = paths::ensure_plans_dir()?;
        Ok(Self { plans_dir, db })
    }

    /// Create a new plan manager with database path (creates new connection)
    ///
    /// Prefer `with_shared_db` when you have a shared database available.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let db = Database::shared(&db_path)?;
        Self::with_shared_db(db)
    }

    /// Get plan for a session (database-backed, no working_dir fallback)
    ///
    /// This is the primary method for loading plans. Plans are strictly
    /// linked to sessions via the database.
    pub fn get_plan(&self, session_id: &str) -> Result<Option<PlanFile>> {
        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);
        store.get_plan_for_session(session_id)
    }

    /// Get the active in-progress plan for a session, filtering out archived plans.
    pub fn get_active_plan(&self, session_id: &str) -> Result<Option<PlanFile>> {
        Ok(self.get_plan(session_id)?.filter(is_active_plan))
    }

    /// Resolve canonical plan lifecycle state for a session.
    pub fn get_lifecycle_state(
        &self,
        session_id: &str,
        work_mode: WorkMode,
    ) -> Result<PlanLifecycleState> {
        Ok(PlanLifecycleState::from_session_mode(
            work_mode,
            self.get_plan(session_id)?,
        ))
    }

    /// Save a plan (creates or updates in database)
    ///
    /// If session already has a plan, it will be replaced.
    pub fn save_plan_for_session(&self, session_id: &str, plan: &PlanFile) -> Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);
        store.upsert_plan(session_id, plan)?;
        Ok(())
    }

    /// Abandon plan for a session (deletes from database)
    pub fn abandon_plan(&self, session_id: &str) -> Result<bool> {
        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);
        store.abandon_plan(session_id)
    }

    /// Check if session has an active plan
    pub fn has_plan(&self, session_id: &str) -> bool {
        let Ok(db) = self.db.lock() else {
            return false;
        };
        let store = PlanStore::new(&db);
        store.has_plan(session_id)
    }

    /// Update plan content for a session
    pub fn update_plan(&self, session_id: &str, plan: &PlanFile) -> Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);
        store.update_content(session_id, plan)
    }

    /// Create a new plan for a session
    ///
    /// The plan is immediately saved to the database.
    /// If the session already has a plan, it will be replaced.
    pub fn create_plan(
        &self,
        title: &str,
        session_id: &str,
        working_dir: Option<&str>,
    ) -> Result<PlanFile> {
        let mut plan = PlanFile::new(title);
        plan.session_id = Some(session_id.to_string());
        plan.working_dir = working_dir.map(|s| s.to_string());

        // Save to database
        self.save_plan_for_session(session_id, &plan)?;

        Ok(plan)
    }

    /// Save a plan to database (legacy API wrapper)
    ///
    /// Uses session_id from the plan. Prefer `save_plan_for_session` when
    /// you have the session_id available.
    pub fn save_plan(&self, plan: &PlanFile) -> Result<()> {
        let session_id = plan
            .session_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Plan has no session_id"))?;
        self.save_plan_for_session(session_id, plan)
    }

    /// List completed plans for a working directory (for history)
    ///
    /// Queries the database for completed plans where the linked session
    /// is in the specified working directory.
    pub fn list_completed_for_dir(&self, working_dir: &str) -> Result<Vec<PlanSummary>> {
        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);

        let all_plans = store.list_all()?;
        Ok(all_plans
            .into_iter()
            .filter(|p| p.status == PlanStatus::Completed)
            .filter(|p| p.working_dir.as_deref() == Some(working_dir))
            .map(|p| PlanSummary {
                id: p.id,
                session_id: Some(p.session_id),
                title: p.title,
                status: p.status,
                progress: (0, 0), // Not stored in summary
                created_at: p.created_at.parse().unwrap_or_else(|_| Utc::now()),
                working_dir: p.working_dir,
            })
            .collect())
    }

    // =========================================================================
    // Legacy file-based methods (for migration only)
    // =========================================================================

    /// Load a plan from disk by path (for migration)
    pub fn load_plan_from_file(&self, path: &PathBuf) -> Result<PlanFile> {
        let content = std::fs::read_to_string(path)?;
        let mut plan = PlanFile::from_markdown(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse plan: {}", e))?;
        plan.file_path = Some(path.clone());
        Ok(plan)
    }

    /// List all plan files on disk (for migration)
    pub fn list_legacy_plans(&self) -> Result<Vec<LegacyPlanSummary>> {
        let mut plans = Vec::new();

        if !self.plans_dir.exists() {
            return Ok(plans);
        }

        for entry in std::fs::read_dir(&self.plans_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "md").unwrap_or(false) {
                match self.load_plan_from_file(&path) {
                    Ok(plan) => {
                        let progress = plan.progress();
                        plans.push(LegacyPlanSummary {
                            path,
                            title: plan.title.clone(),
                            status: plan.status,
                            progress,
                            created_at: plan.created_at,
                            session_id: plan.session_id.clone(),
                            working_dir: plan.working_dir.clone(),
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load plan {:?}: {}", path, e);
                    }
                }
            }
        }

        // Sort by creation date, newest first
        plans.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(plans)
    }

    /// Get the plans directory (for migration)
    pub fn plans_dir(&self) -> &PathBuf {
        &self.plans_dir
    }

    /// Migrate legacy file-based plans to database
    ///
    /// Scans ~/.krusty/plans/*.md and imports plans with valid session_ids
    /// into the database. Returns (migrated, skipped) counts.
    pub fn migrate_legacy_plans(&self) -> Result<(usize, usize)> {
        let legacy_plans = self.list_legacy_plans()?;
        let mut migrated = 0;
        let mut skipped = 0;

        let db = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let store = PlanStore::new(&db);

        for summary in legacy_plans {
            // Only migrate plans that have a session_id
            let Some(session_id) = summary.session_id else {
                tracing::debug!("Skipping plan '{}' - no session_id", summary.title);
                skipped += 1;
                continue;
            };

            // Check if session exists in database
            let session_exists: bool = db
                .conn()
                .query_row(
                    "SELECT 1 FROM sessions WHERE id = ?1",
                    [&session_id],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if !session_exists {
                tracing::debug!(
                    "Skipping plan '{}' - session {} not found",
                    summary.title,
                    session_id
                );
                skipped += 1;
                continue;
            }

            // Check if session already has a plan
            if store.has_plan(&session_id) {
                tracing::debug!(
                    "Skipping plan '{}' - session {} already has a plan",
                    summary.title,
                    session_id
                );
                skipped += 1;
                continue;
            }

            // Load full plan and migrate
            match self.load_plan_from_file(&summary.path) {
                Ok(plan) => {
                    if let Err(e) = store.upsert_plan(&session_id, &plan) {
                        tracing::warn!("Failed to migrate plan '{}': {}", summary.title, e);
                        skipped += 1;
                    } else {
                        tracing::info!("Migrated plan '{}' to database", summary.title);
                        migrated += 1;

                        // Optionally archive the migrated file
                        if let Err(e) = self.archive_legacy_plan(&summary.path) {
                            tracing::warn!("Failed to archive legacy plan: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load legacy plan '{}': {}", summary.title, e);
                    skipped += 1;
                }
            }
        }

        Ok((migrated, skipped))
    }

    /// Archive a legacy plan file by renaming it
    fn archive_legacy_plan(&self, path: &PathBuf) -> Result<()> {
        let archive_dir = self.plans_dir.join("migrated");
        std::fs::create_dir_all(&archive_dir)?;

        if let Some(filename) = path.file_name() {
            let archive_path = archive_dir.join(filename);
            std::fs::rename(path, archive_path)?;
        }

        Ok(())
    }
}

/// Summary of a plan from database
#[derive(Debug, Clone)]
pub struct PlanSummary {
    pub id: String,
    pub session_id: Option<String>,
    pub title: String,
    pub status: PlanStatus,
    pub progress: (usize, usize),
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub working_dir: Option<String>,
}

/// Summary of a legacy file-based plan (for migration)
#[derive(Debug, Clone)]
pub struct LegacyPlanSummary {
    pub path: PathBuf,
    pub title: String,
    pub status: PlanStatus,
    pub progress: (usize, usize),
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub session_id: Option<String>,
    pub working_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn setup_test_manager() -> (PlanManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Create database and run migrations
        let db = Database::new(&db_path).unwrap();

        // Create a test session for the plan
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["session-123", "Test Session", &now, &now],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["session-456", "Test Session 2", &now, &now],
            )
            .unwrap();

        let shared_db = Arc::new(Mutex::new(db));
        let manager = PlanManager {
            plans_dir,
            db: shared_db,
        };
        (manager, temp_dir)
    }

    #[test]
    fn test_create_and_load_plan() {
        let (manager, _temp) = setup_test_manager();

        let plan = manager
            .create_plan("Test Plan", "session-123", Some("/tmp/test"))
            .unwrap();

        assert_eq!(plan.title, "Test Plan");
        assert_eq!(plan.session_id, Some("session-123".to_string()));

        // Reload and verify
        let loaded = manager.get_plan("session-123").unwrap().unwrap();
        assert_eq!(loaded.title, "Test Plan");
        assert_eq!(loaded.session_id, Some("session-123".to_string()));
    }

    #[test]
    fn test_plan_per_session() {
        let (manager, _temp) = setup_test_manager();

        manager.create_plan("Plan A", "session-123", None).unwrap();
        manager.create_plan("Plan B", "session-456", None).unwrap();

        // Each session has its own plan
        let plan_a = manager.get_plan("session-123").unwrap().unwrap();
        let plan_b = manager.get_plan("session-456").unwrap().unwrap();

        assert_eq!(plan_a.title, "Plan A");
        assert_eq!(plan_b.title, "Plan B");

        // No plan for non-existent session
        assert!(manager.get_plan("session-999").unwrap().is_none());
    }

    #[test]
    fn test_save_with_changes() {
        let (manager, _temp) = setup_test_manager();

        let mut plan = manager.create_plan("Test", "session-123", None).unwrap();
        {
            let phase = plan.add_phase("Phase 1");
            phase.add_task("Task one");
        }
        plan.check_task("1.1");
        manager.save_plan(&plan).unwrap();

        // Reload and verify
        let loaded = manager.get_plan("session-123").unwrap().unwrap();
        assert!(loaded.find_task("1.1").unwrap().completed);
    }

    #[test]
    fn test_abandon_plan() {
        let (manager, _temp) = setup_test_manager();

        manager.create_plan("Test", "session-123", None).unwrap();
        assert!(manager.has_plan("session-123"));

        manager.abandon_plan("session-123").unwrap();
        assert!(!manager.has_plan("session-123"));
    }

    #[test]
    fn test_get_active_plan_filters_completed_plan() {
        let (manager, _temp) = setup_test_manager();

        let mut plan = manager.create_plan("Test", "session-123", None).unwrap();
        let phase = plan.add_phase("Phase 1");
        phase.add_task("Done task");
        plan.complete_task("1.1", "Implemented").unwrap();
        manager.save_plan(&plan).unwrap();

        assert!(manager.get_active_plan("session-123").unwrap().is_none());
        assert!(manager.get_plan("session-123").unwrap().is_some());
    }
}
