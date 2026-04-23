use anyhow::Result;
use std::path::PathBuf;

use super::{LegacyPlanSummary, PlanManager};
use crate::plan::PlanFile;
use crate::storage::PlanStore;

impl PlanManager {
    /// Load a plan from disk by path (for migration).
    pub fn load_plan_from_file(&self, path: &PathBuf) -> Result<PlanFile> {
        let content = std::fs::read_to_string(path)?;
        let mut plan = PlanFile::from_markdown(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse plan: {}", e))?;
        plan.file_path = Some(path.clone());
        Ok(plan)
    }

    /// List all plan files on disk (for migration).
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

        plans.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(plans)
    }

    /// Get the plans directory (for migration).
    pub fn plans_dir(&self) -> &PathBuf {
        &self.plans_dir
    }

    /// Migrate legacy file-based plans to database.
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
            let Some(session_id) = summary.session_id else {
                tracing::debug!("Skipping plan '{}' - no session_id", summary.title);
                skipped += 1;
                continue;
            };

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

            if store.has_plan(&session_id) {
                tracing::debug!(
                    "Skipping plan '{}' - session {} already has a plan",
                    summary.title,
                    session_id
                );
                skipped += 1;
                continue;
            }

            match self.load_plan_from_file(&summary.path) {
                Ok(plan) => {
                    if let Err(e) = store.upsert_plan(&session_id, &plan) {
                        tracing::warn!("Failed to migrate plan '{}': {}", summary.title, e);
                        skipped += 1;
                    } else {
                        tracing::info!("Migrated plan '{}' to database", summary.title);
                        migrated += 1;

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
