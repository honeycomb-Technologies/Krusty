use anyhow::{bail, Context, Result};

use super::{
    model::{
        PackageHookConfig, PackageHookLoadReport, UserHook, UserHookSource, UserHookType,
        DEFAULT_HOOK_TIMEOUT_SECONDS,
    },
    package::load_package_hooks,
};

/// Manager for user hooks. Handles CRUD and persistence.
pub struct UserHookManager {
    hooks: Vec<UserHook>,
}

fn persisted_hook_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserHook> {
    Ok(UserHook {
        id: row.get(0)?,
        hook_type: UserHookType::parse(&row.get::<_, String>(1)?)
            .unwrap_or(UserHookType::PreToolUse),
        tool_pattern: row.get(2)?,
        command: row.get(3)?,
        enabled: row.get::<_, i32>(4)? != 0,
        timeout_seconds: DEFAULT_HOOK_TIMEOUT_SECONDS,
        created_at: row.get(5)?,
        source: UserHookSource::User,
        owner_user_id: row.get(6)?,
        working_dir: None,
        compiled_pattern: None,
    })
}

impl Default for UserHookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UserHookManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Load hooks from database (legacy - no user filtering).
    pub fn load(&mut self, db: &crate::storage::Database) -> Result<()> {
        self.load_for_user(db, None)
    }

    /// Load hooks for a specific user (multi-tenant) or all hooks (single-tenant).
    pub fn load_for_user(
        &mut self,
        db: &crate::storage::Database,
        user_id: Option<&str>,
    ) -> Result<()> {
        use rusqlite::params;

        let conn = db.conn();
        let hooks: Vec<UserHook> = if let Some(uid) = user_id {
            let mut stmt = conn.prepare(
                "SELECT id, hook_type, tool_pattern, command, enabled, created_at, user_id
                 FROM user_hooks WHERE user_id = ?1 OR user_id IS NULL ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![uid], persisted_hook_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, hook_type, tool_pattern, command, enabled, created_at, user_id
                 FROM user_hooks ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![], persisted_hook_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        // Database refreshes replace only persisted hooks. Package hooks are
        // ephemeral runtime contributions managed by `replace_package_hooks`.
        self.hooks.retain(UserHook::is_package_hook);
        for mut hook in hooks {
            hook.compile_pattern();
            self.hooks.push(hook);
        }

        Ok(())
    }

    /// Save a new hook to database (legacy - no user_id).
    pub fn save(&mut self, db: &crate::storage::Database, hook: UserHook) -> Result<()> {
        self.save_for_user(db, hook, None)
    }

    /// Save a new hook for a specific user.
    pub fn save_for_user(
        &mut self,
        db: &crate::storage::Database,
        mut hook: UserHook,
        user_id: Option<&str>,
    ) -> Result<()> {
        use rusqlite::params;

        if hook.is_package_hook() {
            bail!("package hooks are read-only and cannot be persisted");
        }
        hook.owner_user_id = user_id.map(str::to_owned);

        db.conn().execute(
            "INSERT INTO user_hooks (id, hook_type, tool_pattern, command, enabled, created_at, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &hook.id,
                hook.hook_type.display_name(),
                &hook.tool_pattern,
                &hook.command,
                if hook.enabled { 1 } else { 0 },
                &hook.created_at,
                hook.owner_user_id.as_deref(),
            ],
        )?;

        hook.compile_pattern();
        self.hooks.push(hook);
        Ok(())
    }

    /// Delete a hook by ID (validates ownership in multi-tenant mode).
    pub fn delete(&mut self, db: &crate::storage::Database, id: &str) -> Result<()> {
        self.delete_for_user(db, id, None)
    }

    /// Delete a hook for a specific user.
    pub fn delete_for_user(
        &mut self,
        db: &crate::storage::Database,
        id: &str,
        user_id: Option<&str>,
    ) -> Result<()> {
        use rusqlite::params;

        let hook = self
            .hooks
            .iter()
            .find(|hook| hook.id == id)
            .with_context(|| format!("hook '{id}' not found for current user"))?;
        if hook.is_package_hook() {
            bail!("package hooks are read-only and cannot be deleted");
        }
        if hook.owner_user_id() != user_id {
            bail!("hook '{id}' not found for current user");
        }

        let changed = if let Some(uid) = user_id {
            db.conn().execute(
                "DELETE FROM user_hooks WHERE id = ?1 AND user_id = ?2",
                params![id, uid],
            )?
        } else {
            db.conn().execute(
                "DELETE FROM user_hooks WHERE id = ?1 AND user_id IS NULL",
                params![id],
            )?
        };
        if changed == 0 {
            bail!("hook '{id}' not found for current user");
        }
        self.hooks.retain(|h| h.id != id);
        Ok(())
    }

    /// Toggle a hook's enabled state (validates ownership in multi-tenant mode).
    pub fn toggle(&mut self, db: &crate::storage::Database, id: &str) -> Result<bool> {
        self.toggle_for_user(db, id, None)
    }

    /// Toggle a hook for a specific user.
    pub fn toggle_for_user(
        &mut self,
        db: &crate::storage::Database,
        id: &str,
        user_id: Option<&str>,
    ) -> Result<bool> {
        use rusqlite::params;

        let hook = self
            .hooks
            .iter()
            .find(|hook| hook.id == id)
            .with_context(|| format!("hook '{id}' not found for current user"))?;
        if hook.is_package_hook() {
            bail!("package hooks are read-only and cannot be toggled");
        }
        if hook.owner_user_id() != user_id {
            bail!("hook '{id}' not found for current user");
        }
        let enabled = !hook.enabled;
        let changed = if let Some(uid) = user_id {
            db.conn().execute(
                "UPDATE user_hooks SET enabled = ?1 WHERE id = ?2 AND user_id = ?3",
                params![if enabled { 1 } else { 0 }, id, uid],
            )?
        } else {
            db.conn().execute(
                "UPDATE user_hooks SET enabled = ?1 WHERE id = ?2 AND user_id IS NULL",
                params![if enabled { 1 } else { 0 }, id],
            )?
        };
        if changed == 0 {
            bail!("hook '{id}' not found for current user");
        }
        if let Some(hook) = self.hooks.iter_mut().find(|hook| hook.id == id) {
            hook.enabled = enabled;
        }
        Ok(enabled)
    }

    /// Replace every ephemeral package hook in one fail-closed operation.
    ///
    /// All files are parsed and validated before the new hooks become visible.
    /// If any input is invalid, existing package hooks are removed so a
    /// disabled/uninstalled package can never continue executing from stale
    /// in-memory state. Persisted user hooks are never modified.
    pub fn replace_package_hooks(
        &mut self,
        configs: Vec<PackageHookConfig>,
    ) -> Result<PackageHookLoadReport> {
        let config_count = configs.len();
        let loaded = match load_package_hooks(&configs) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.clear_package_hooks();
                return Err(error).context("package hook replacement failed closed");
            }
        };
        let hook_count = loaded.len();
        self.clear_package_hooks();
        self.hooks.extend(loaded);
        Ok(PackageHookLoadReport {
            config_count,
            hook_count,
        })
    }

    /// Remove every ephemeral package hook, preserving user-created hooks.
    pub fn clear_package_hooks(&mut self) {
        self.hooks.retain(|hook| !hook.is_package_hook());
    }

    /// Get only package-contributed hooks.
    pub fn package_hooks(&self) -> Vec<&UserHook> {
        self.hooks
            .iter()
            .filter(|hook| hook.is_package_hook())
            .collect()
    }

    /// Get all hooks.
    pub fn hooks(&self) -> &[UserHook] {
        &self.hooks
    }

    /// Get global/package hooks plus persisted hooks owned by `user_id`.
    /// Tenant-owned hooks belonging to any other user are never returned.
    pub fn hooks_for_user(&self, user_id: Option<&str>) -> Vec<&UserHook> {
        self.hooks
            .iter()
            .filter(|hook| hook.applies_to_user(user_id))
            .collect()
    }

    /// Get hooks by type.
    pub fn hooks_by_type(&self, hook_type: UserHookType) -> Vec<&UserHook> {
        self.hooks
            .iter()
            .filter(|h| h.hook_type == hook_type)
            .collect()
    }

    /// Get enabled hooks that match a tool name.
    pub fn matching_hooks(&mut self, hook_type: UserHookType, tool_name: &str) -> Vec<&UserHook> {
        self.matching_hooks_for_user(hook_type, tool_name, None)
    }

    /// Get enabled matching global/package hooks and hooks owned by `user_id`.
    pub fn matching_hooks_for_user(
        &mut self,
        hook_type: UserHookType,
        tool_name: &str,
        user_id: Option<&str>,
    ) -> Vec<&UserHook> {
        let mut matching_indices = Vec::new();
        for (idx, hook) in self.hooks.iter_mut().enumerate() {
            if hook.hook_type == hook_type
                && hook.applies_to_user(user_id)
                && hook.matches(tool_name)
            {
                matching_indices.push(idx);
            }
        }

        matching_indices
            .into_iter()
            .filter_map(|idx| self.hooks.get(idx))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::UserHookManager;
    use crate::agent::user_hooks::{PackageHookConfig, UserHook, UserHookType};
    use crate::storage::Database;

    fn create_test_hook(hook_type: UserHookType, pattern: &str, command: &str) -> UserHook {
        UserHook::new(hook_type, pattern.to_string(), command.to_string())
    }

    #[test]
    fn test_user_hook_manager_operations() {
        let mut manager = UserHookManager::new();

        assert_eq!(manager.hooks().len(), 0);

        let hook1 = create_test_hook(UserHookType::PreToolUse, "Write", "echo '1'");
        let hook2 = create_test_hook(UserHookType::PostToolUse, "Read", "echo '2'");

        manager.hooks.push(hook1);
        manager.hooks.push(hook2);

        assert_eq!(manager.hooks().len(), 2);
    }

    #[test]
    fn test_user_hook_manager_matching_hooks() {
        let mut manager = UserHookManager::new();

        let hook1 = create_test_hook(UserHookType::PreToolUse, "Write", "echo '1'");
        let hook2 = create_test_hook(UserHookType::PreToolUse, "Read", "echo '2'");
        let hook3 = create_test_hook(UserHookType::PostToolUse, "Write", "echo '3'");
        let hook4 = create_test_hook(UserHookType::PreToolUse, ".*", "echo '4'");

        manager.hooks.push(hook1);
        manager.hooks.push(hook2);
        manager.hooks.push(hook3);
        manager.hooks.push(hook4);

        let matching = manager.matching_hooks(UserHookType::PreToolUse, "Write");
        assert_eq!(matching.len(), 2);

        let matching = manager.matching_hooks(UserHookType::PreToolUse, "Read");
        assert_eq!(matching.len(), 2);

        let matching = manager.matching_hooks(UserHookType::PostToolUse, "Write");
        assert_eq!(matching.len(), 1);
    }

    #[test]
    fn test_user_hook_manager_no_matching_hooks() {
        let mut manager = UserHookManager::new();

        let hook1 = create_test_hook(UserHookType::PreToolUse, "Write", "echo '1'");
        let hook2 = create_test_hook(UserHookType::PostToolUse, "Read", "echo '2'");

        manager.hooks.push(hook1);
        manager.hooks.push(hook2);

        let matching = manager.matching_hooks(UserHookType::PreToolUse, "Bash");
        assert_eq!(matching.len(), 0);
    }

    #[test]
    fn tenant_hooks_are_loaded_matched_and_mutated_only_for_their_owner() {
        let temp = tempfile::tempdir().expect("temporary database directory");
        let db = Database::new(&temp.path().join("hooks.db")).expect("initialize database");
        for user in ["alice", "bob"] {
            db.conn()
                .execute(
                    "INSERT INTO users (id, email) VALUES (?1, ?2)",
                    rusqlite::params![user, format!("{user}@example.test")],
                )
                .expect("insert test user");
        }

        let mut manager = UserHookManager::new();
        let global = create_test_hook(UserHookType::PreToolUse, ".*", "global");
        let global_id = global.id.clone();
        manager.save(&db, global).expect("save global hook");

        let alice = create_test_hook(UserHookType::PreToolUse, ".*", "alice");
        let alice_id = alice.id.clone();
        manager
            .save_for_user(&db, alice, Some("alice"))
            .expect("save Alice hook");

        let bob = create_test_hook(UserHookType::PreToolUse, ".*", "bob");
        let bob_id = bob.id.clone();
        manager
            .save_for_user(&db, bob, Some("bob"))
            .expect("save Bob hook");

        let alice_visible = manager
            .hooks_for_user(Some("alice"))
            .into_iter()
            .map(|hook| hook.id.as_str())
            .collect::<Vec<_>>();
        assert!(alice_visible.contains(&global_id.as_str()));
        assert!(alice_visible.contains(&alice_id.as_str()));
        assert!(!alice_visible.contains(&bob_id.as_str()));

        let alice_matching = manager
            .matching_hooks_for_user(UserHookType::PreToolUse, "Bash", Some("alice"))
            .into_iter()
            .map(|hook| hook.command.as_str())
            .collect::<Vec<_>>();
        assert_eq!(alice_matching, vec!["global", "alice"]);

        assert!(
            manager
                .delete_for_user(&db, &alice_id, Some("bob"))
                .is_err(),
            "Bob must not delete Alice's hook"
        );
        assert!(
            manager
                .toggle_for_user(&db, &global_id, Some("alice"))
                .is_err(),
            "tenant requests must not mutate global hooks"
        );
        assert!(
            manager.delete_for_user(&db, &bob_id, None).is_err(),
            "unscoped local requests must not mutate tenant hooks"
        );
        manager
            .delete_for_user(&db, &alice_id, Some("alice"))
            .expect("Alice may delete her own hook");

        let mut alice_snapshot = UserHookManager::new();
        alice_snapshot
            .load_for_user(&db, Some("alice"))
            .expect("load scoped hook snapshot");
        let snapshot_ids = alice_snapshot
            .hooks()
            .iter()
            .map(|hook| hook.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(snapshot_ids, vec![global_id.as_str()]);
        assert!(!snapshot_ids.contains(&bob_id.as_str()));
    }

    #[test]
    fn package_hook_replacement_is_ephemeral_and_fail_closed() {
        let temp = tempfile::tempdir().expect("temp package root");
        let config_path = temp.path().join("hooks.json");
        std::fs::write(
            &config_path,
            r#"{
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "./guard.sh"}]
                    }]
                }
            }"#,
        )
        .expect("write hook config");

        let mut manager = UserHookManager::new();
        manager
            .hooks
            .push(create_test_hook(UserHookType::PostToolUse, "Read", "true"));
        let report = manager
            .replace_package_hooks(vec![PackageHookConfig::new(
                "acme.guard",
                &config_path,
                temp.path(),
            )])
            .expect("package hooks should load");

        assert_eq!(report.config_count, 1);
        assert_eq!(report.hook_count, 1);
        assert_eq!(manager.hooks().len(), 2);
        let package_hook = manager.package_hooks()[0];
        assert_eq!(package_hook.source.plugin_id(), Some("acme.guard"));
        let canonical_root = temp.path().canonicalize().expect("canonical package root");
        assert_eq!(package_hook.working_dir(), Some(canonical_root.as_path()));

        std::fs::write(&config_path, "{not valid").expect("replace config with invalid data");
        manager
            .replace_package_hooks(vec![PackageHookConfig::new(
                "acme.guard",
                &config_path,
                temp.path(),
            )])
            .expect_err("invalid replacement must fail closed");

        assert!(manager.package_hooks().is_empty());
        assert_eq!(manager.hooks().len(), 1, "persisted hooks must survive");
    }
}
