use anyhow::Result;
use rusqlite::params;
use uuid::Uuid;

use crate::storage::database::Database;

use super::model::{AgentMemory, MemoryType};
use super::query::{build_list_query, row_to_memory};

pub struct MemoryStore {
    db: Database,
}

impl MemoryStore {
    /// Open or create a memory store at the given database path.
    ///
    /// Creates the `agent_memories` table and indexes if they don't exist.
    pub fn new(db: Database) -> Self {
        let _ = db.conn().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS agent_memories (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL CHECK(memory_type IN ('user', 'feedback', 'project', 'reference')),
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                project_dir TEXT,
                user_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_agent_memories_type ON agent_memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_project ON agent_memories(project_dir);
            CREATE INDEX IF NOT EXISTS idx_agent_memories_user ON agent_memories(user_id);
            "#,
        );
        Self { db }
    }

    /// Save a new memory. Generates an id if not already set.
    pub fn save(
        &self,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<AgentMemory> {
        let id = Uuid::new_v4().to_string();
        self.db.conn().execute(
            "INSERT INTO agent_memories (id, memory_type, title, content, project_dir, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                memory_type.as_str(),
                title,
                content,
                project_dir,
                user_id
            ],
        )?;

        // Read back the created record to get server-side defaults
        self.get(&id)?
            .ok_or_else(|| anyhow::anyhow!("memory not found after insert"))
    }

    /// Get a single memory by id.
    pub fn get(&self, id: &str) -> Result<Option<AgentMemory>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
             FROM agent_memories WHERE id = ?1",
        )?;
        let memory = stmt
            .query_row(params![id], |row| Ok(row_to_memory(row)))
            .ok();
        Ok(memory)
    }

    /// Update an existing memory's title and/or content.
    pub fn update(&self, id: &str, title: Option<&str>, content: Option<&str>) -> Result<()> {
        match (title, content) {
            (Some(t), Some(c)) => {
                self.db.conn().execute(
                    "UPDATE agent_memories SET title = ?1, content = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![t, c, id],
                )?;
            }
            (Some(t), None) => {
                self.db.conn().execute(
                    "UPDATE agent_memories SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![t, id],
                )?;
            }
            (None, Some(c)) => {
                self.db.conn().execute(
                    "UPDATE agent_memories SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![c, id],
                )?;
            }
            (None, None) => {}
        }
        Ok(())
    }

    /// Delete a memory by id.
    pub fn delete(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM agent_memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// List all memories, optionally filtered by project_dir and/or user_id.
    ///
    /// When `project_dir` is `Some`, returns both project-scoped and global memories.
    pub fn list(&self, project_dir: Option<&str>, user_id: Option<&str>) -> Vec<AgentMemory> {
        let (sql, bound) = build_list_query(None, project_dir, user_id);
        self.query_memories(&sql, &bound)
    }

    /// List memories of a specific type, optionally filtered by project_dir and/or user_id.
    pub fn list_by_type(
        &self,
        memory_type: MemoryType,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Vec<AgentMemory> {
        let (sql, bound) = build_list_query(Some(memory_type), project_dir, user_id);
        self.query_memories(&sql, &bound)
    }

    /// Find a memory by exact title within a project scope.
    pub fn find_by_title(&self, title: &str, project_dir: Option<&str>) -> Option<AgentMemory> {
        if let Some(pd) = project_dir {
            let mut stmt = self.db.conn().prepare(
                "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
                 FROM agent_memories WHERE title = ?1 AND (project_dir = ?2 OR project_dir IS NULL)
                 ORDER BY updated_at DESC LIMIT 1",
            ).ok()?;
            stmt.query_row(params![title, pd], |row| Ok(row_to_memory(row)))
                .ok()
        } else {
            let mut stmt = self.db.conn().prepare(
                "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
                 FROM agent_memories WHERE title = ?1 AND project_dir IS NULL
                 ORDER BY updated_at DESC LIMIT 1",
            ).ok()?;
            stmt.query_row(params![title], |row| Ok(row_to_memory(row)))
                .ok()
        }
    }

    /// Find a memory by exact title within project/user scope.
    ///
    /// When `project_dir` is set, both project-scoped and global memories are
    /// considered. When `user_id` is set, both user-scoped and global memories
    /// are considered.
    pub fn find_by_title_for_user(
        &self,
        title: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<AgentMemory> {
        let mut sql = String::from(
            "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
             FROM agent_memories
             WHERE title = ?1",
        );
        let mut bound = vec![title.to_string()];

        if let Some(pd) = project_dir {
            bound.push(pd.to_string());
            sql.push_str(&format!(
                " AND (project_dir = ?{} OR project_dir IS NULL)",
                bound.len()
            ));
        } else {
            sql.push_str(" AND project_dir IS NULL");
        }

        if let Some(uid) = user_id {
            bound.push(uid.to_string());
            sql.push_str(&format!(
                " AND (user_id = ?{} OR user_id IS NULL)",
                bound.len()
            ));
        } else {
            sql.push_str(" AND user_id IS NULL");
        }

        sql.push_str(" ORDER BY updated_at DESC LIMIT 1");

        let mut stmt = self.db.conn().prepare(&sql).ok()?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        stmt.query_row(params.as_slice(), |row| Ok(row_to_memory(row)))
            .ok()
    }

    /// Save a new memory or update an existing exact-title match within the
    /// same effective scope.
    ///
    /// Returns the memory plus a flag indicating whether it was newly created.
    pub fn save_or_update_by_title(
        &self,
        memory_type: MemoryType,
        title: &str,
        content: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<(AgentMemory, bool)> {
        if let Some(existing) = self.find_by_title_in_exact_scope(title, project_dir, user_id) {
            self.update(&existing.id, Some(title), Some(content))?;
            let updated = self
                .get(&existing.id)?
                .ok_or_else(|| anyhow::anyhow!("memory not found after update"))?;
            return Ok((updated, false));
        }

        let created = self.save(memory_type, title, content, project_dir, user_id)?;
        Ok((created, true))
    }

    pub fn find_by_title_in_exact_scope(
        &self,
        title: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<AgentMemory> {
        let mut sql = String::from(
            "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
             FROM agent_memories
             WHERE title = ?1",
        );
        let mut bound = vec![title.to_string()];

        if let Some(pd) = project_dir {
            bound.push(pd.to_string());
            sql.push_str(&format!(" AND project_dir = ?{}", bound.len()));
        } else {
            sql.push_str(" AND project_dir IS NULL");
        }

        if let Some(uid) = user_id {
            bound.push(uid.to_string());
            sql.push_str(&format!(" AND user_id = ?{}", bound.len()));
        } else {
            sql.push_str(" AND user_id IS NULL");
        }

        sql.push_str(" ORDER BY updated_at DESC LIMIT 1");

        let mut stmt = self.db.conn().prepare(&sql).ok()?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        stmt.query_row(params.as_slice(), |row| Ok(row_to_memory(row)))
            .ok()
    }

    fn query_memories(&self, sql: &str, bound: &[String]) -> Vec<AgentMemory> {
        let mut stmt = match self.db.conn().prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Memory query failed (prepare): {}", e);
                return Vec::new();
            }
        };
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = match stmt.query_map(params.as_slice(), |row| Ok(row_to_memory(row))) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Memory query failed (execute): {}", e);
                return Vec::new();
            }
        };
        rows.filter_map(|r| r.ok()).collect()
    }
}
