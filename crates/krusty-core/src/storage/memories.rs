//! Persistent agent memory storage
//!
//! Cross-session knowledge retention for user preferences, feedback,
//! project context, and external references. SQLite-backed with
//! multi-tenant and project-scoped support.

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::Database;

/// Memory type taxonomy (matching Claude Code's categories)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Role, expertise, preferences, goals
    User,
    /// Corrections and confirmed approaches
    Feedback,
    /// Ongoing work, deadlines, decisions
    Project,
    /// External system pointers (issue trackers, dashboards)
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "feedback" => Some(Self::Feedback),
            "project" => Some(Self::Project),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single persistent memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub project_dir: Option<String>,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Persistent memory store backed by the shared SQLite database
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
            params![id, memory_type.as_str(), title, content, project_dir, user_id],
        )?;

        // Read back the created record to get server-side defaults
        self.get(&id)?.ok_or_else(|| anyhow::anyhow!("memory not found after insert"))
    }

    /// Get a single memory by id.
    pub fn get(&self, id: &str) -> Result<Option<AgentMemory>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
             FROM agent_memories WHERE id = ?1",
        )?;
        let memory = stmt
            .query_row(params![id], |row| {
                Ok(row_to_memory(row))
            })
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
        self.db.conn().execute(
            "DELETE FROM agent_memories WHERE id = ?1",
            params![id],
        )?;
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
            stmt.query_row(params![title, pd], |row| Ok(row_to_memory(row))).ok()
        } else {
            let mut stmt = self.db.conn().prepare(
                "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
                 FROM agent_memories WHERE title = ?1 AND project_dir IS NULL
                 ORDER BY updated_at DESC LIMIT 1",
            ).ok()?;
            stmt.query_row(params![title], |row| Ok(row_to_memory(row))).ok()
        }
    }

    fn query_memories(&self, sql: &str, bound: &[String]) -> Vec<AgentMemory> {
        let mut stmt = match self.db.conn().prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let params: Vec<&dyn rusqlite::types::ToSql> =
            bound.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let rows = match stmt.query_map(params.as_slice(), |row| Ok(row_to_memory(row))) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }
}

fn row_to_memory(row: &rusqlite::Row) -> AgentMemory {
    let type_str: String = row.get(1).unwrap_or_default();
    AgentMemory {
        id: row.get(0).unwrap_or_default(),
        memory_type: MemoryType::from_str(&type_str).unwrap_or(MemoryType::Project),
        title: row.get(2).unwrap_or_default(),
        content: row.get(3).unwrap_or_default(),
        project_dir: row.get(4).ok(),
        user_id: row.get(5).ok(),
        created_at: row.get(6).unwrap_or_default(),
        updated_at: row.get(7).unwrap_or_default(),
    }
}

/// Build a parameterized list query with optional type/project/user filters.
///
/// Returns the SQL string and a vector of owned `String` values that the caller
/// must keep alive while binding.  The companion `to_params` helper converts
/// the `Vec<String>` to `&[&dyn ToSql]`.
fn build_list_query(
    memory_type: Option<MemoryType>,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> (String, Vec<String>) {
    let mut sql = String::from(
        "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
         FROM agent_memories WHERE 1=1",
    );
    let mut bound: Vec<String> = Vec::new();

    if let Some(mt) = memory_type {
        bound.push(mt.as_str().to_string());
        sql.push_str(&format!(" AND memory_type = ?{}", bound.len()));
    }

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
    }

    sql.push_str(" ORDER BY updated_at DESC");
    (sql, bound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_store() -> (MemoryStore, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("memories.db");
        let db = Database::new(&db_path).expect("database");
        (MemoryStore::new(db), temp_dir)
    }

    #[test]
    fn save_and_list_memories() {
        let (store, _tmp) = create_store();
        store
            .save(MemoryType::User, "Role", "Backend developer", None, None)
            .unwrap();
        store
            .save(MemoryType::Feedback, "Testing", "Use integration tests", None, None)
            .unwrap();

        let all = store.list(None, None);
        assert_eq!(all.len(), 2);

        let users = store.list_by_type(MemoryType::User, None, None);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].title, "Role");
    }

    #[test]
    fn update_memory() {
        let (store, _tmp) = create_store();
        let mem = store
            .save(MemoryType::Project, "Sprint", "Sprint 4", None, None)
            .unwrap();
        store.update(&mem.id, Some("Sprint goal"), Some("Ship auth")).unwrap();

        let updated = store.get(&mem.id).unwrap().unwrap();
        assert_eq!(updated.title, "Sprint goal");
        assert_eq!(updated.content, "Ship auth");
    }

    #[test]
    fn delete_memory() {
        let (store, _tmp) = create_store();
        let mem = store
            .save(MemoryType::Reference, "Tracker", "Linear INGEST", None, None)
            .unwrap();
        assert!(store.get(&mem.id).unwrap().is_some());
        store.delete(&mem.id).unwrap();
        assert!(store.get(&mem.id).unwrap().is_none());
    }

    #[test]
    fn project_scoped_memories() {
        let (store, _tmp) = create_store();
        store
            .save(MemoryType::Project, "Global", "applies everywhere", None, None)
            .unwrap();
        store
            .save(
                MemoryType::Project,
                "Scoped",
                "only in project-a",
                Some("/home/user/project-a"),
                None,
            )
            .unwrap();

        // Listing with project_dir returns both scoped + global
        let project_a = store.list(Some("/home/user/project-a"), None);
        assert_eq!(project_a.len(), 2);

        // Listing without project_dir returns only global
        let global = store.list(None, None);
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].title, "Global");
    }

    #[test]
    fn find_by_title() {
        let (store, _tmp) = create_store();
        store
            .save(MemoryType::User, "Preferred language", "Rust", None, None)
            .unwrap();
        let found = store.find_by_title("Preferred language", None);
        assert!(found.is_some());
        assert_eq!(found.unwrap().content, "Rust");

        let missing = store.find_by_title("Nonexistent", None);
        assert!(missing.is_none());
    }
}
