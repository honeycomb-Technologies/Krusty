use anyhow::Result;

use super::model::{UserHook, UserHookType};

/// Manager for user hooks. Handles CRUD and persistence.
pub struct UserHookManager {
    hooks: Vec<UserHook>,
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
                "SELECT id, hook_type, tool_pattern, command, enabled, created_at
                 FROM user_hooks WHERE user_id = ?1 OR user_id IS NULL ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![uid], |row| {
                Ok(UserHook {
                    id: row.get(0)?,
                    hook_type: UserHookType::parse(&row.get::<_, String>(1)?)
                        .unwrap_or(UserHookType::PreToolUse),
                    tool_pattern: row.get(2)?,
                    command: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    compiled_pattern: None,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, hook_type, tool_pattern, command, enabled, created_at
                 FROM user_hooks ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![], |row| {
                Ok(UserHook {
                    id: row.get(0)?,
                    hook_type: UserHookType::parse(&row.get::<_, String>(1)?)
                        .unwrap_or(UserHookType::PreToolUse),
                    tool_pattern: row.get(2)?,
                    command: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    compiled_pattern: None,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        self.hooks.clear();
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
        hook: UserHook,
        user_id: Option<&str>,
    ) -> Result<()> {
        use rusqlite::params;

        db.conn().execute(
            "INSERT INTO user_hooks (id, hook_type, tool_pattern, command, enabled, created_at, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                hook.id,
                hook.hook_type.display_name(),
                hook.tool_pattern,
                hook.command,
                if hook.enabled { 1 } else { 0 },
                hook.created_at,
                user_id,
            ],
        )?;

        let mut h = hook;
        h.compile_pattern();
        self.hooks.push(h);
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

        if let Some(uid) = user_id {
            db.conn().execute(
                "DELETE FROM user_hooks WHERE id = ?1 AND (user_id = ?2 OR user_id IS NULL)",
                params![id, uid],
            )?;
        } else {
            db.conn()
                .execute("DELETE FROM user_hooks WHERE id = ?1", params![id])?;
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

        let hook = self.hooks.iter_mut().find(|h| h.id == id);
        if let Some(h) = hook {
            h.enabled = !h.enabled;
            if let Some(uid) = user_id {
                db.conn().execute(
                    "UPDATE user_hooks SET enabled = ?1 WHERE id = ?2 AND (user_id = ?3 OR user_id IS NULL)",
                    params![if h.enabled { 1 } else { 0 }, id, uid],
                )?;
            } else {
                db.conn().execute(
                    "UPDATE user_hooks SET enabled = ?1 WHERE id = ?2",
                    params![if h.enabled { 1 } else { 0 }, id],
                )?;
            }
            return Ok(h.enabled);
        }
        Ok(false)
    }

    /// Get all hooks.
    pub fn hooks(&self) -> &[UserHook] {
        &self.hooks
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
        let mut matching_indices = Vec::new();
        for (idx, hook) in self.hooks.iter_mut().enumerate() {
            if hook.hook_type == hook_type && hook.matches(tool_name) {
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
    use crate::agent::user_hooks::{UserHook, UserHookType};

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
}
