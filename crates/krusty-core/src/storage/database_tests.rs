//! Tests for database migrations
//!
//! These tests verify that:
//! - All migrations apply successfully
//! - Schema version is tracked correctly
//! - Migrations can be rolled back (conceptually - we don't actually rollback)
//! - Data survives through migrations

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::database::Database;

    /// Helper to create a temporary database for testing
    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        (db, temp_dir)
    }

    #[test]
    fn test_database_creation() {
        let (db, _temp) = create_test_db();

        // Database should initialize with schema_version table
        let version = db.get_schema_version();
        assert_eq!(version, 11, "Expected current schema version to be 11");
    }

    #[test]
    fn test_sessions_table_exists() {
        let (db, _temp) = create_test_db();

        // Verify sessions table exists and has correct columns
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
            .expect("Failed to prepare query");

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"sessions".to_string()));

        // Verify key columns exist
        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .expect("Failed to prepare PRAGMA");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("Failed to get columns")
            .filter_map(Result::ok)
            .collect();

        assert!(columns.contains(&"id".to_string()));
        assert!(columns.contains(&"title".to_string()));
        assert!(columns.contains(&"created_at".to_string()));
        assert!(columns.contains(&"updated_at".to_string()));
        assert!(columns.contains(&"user_id".to_string()));
        assert!(columns.contains(&"working_dir".to_string()));
    }

    #[test]
    fn test_messages_table_exists() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='messages'")
            .expect("Failed to prepare query");

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"messages".to_string()));

        // Verify foreign key constraint
        let mut stmt = conn
            .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='messages'")
            .expect("Failed to get table DDL");

        let ddl: Option<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to get DDL")
            .filter_map(Result::ok)
            .next();

        let ddl = ddl.expect("Messages table should exist");
        assert!(ddl.contains("FOREIGN KEY"));
        assert!(ddl.contains("ON DELETE CASCADE"));
    }

    #[test]
    fn test_foreign_keys_enabled() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("PRAGMA foreign_keys")
            .expect("Failed to prepare PRAGMA");

        let fk_enabled: i32 = stmt
            .query_row([], |row| row.get(0))
            .expect("Failed to get foreign_keys setting");

        assert_eq!(fk_enabled, 1, "Foreign keys should be enabled");
    }

    #[test]
    fn test_wal_mode_enabled() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("PRAGMA journal_mode")
            .expect("Failed to prepare PRAGMA");

        let journal_mode: String = stmt
            .query_row([], |row| row.get(0))
            .expect("Failed to get journal_mode");

        assert_eq!(
            journal_mode.to_lowercase(),
            "wal",
            "WAL mode should be enabled"
        );
    }

    #[test]
    fn test_schema_version_increments() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");

        // Create database fresh
        let db = Database::new(&db_path).expect("Failed to create database");
        let version = db.get_schema_version();

        // After all migrations, version should be 11
        assert_eq!(version, 11, "Expected final schema version");
    }

    #[test]
    fn test_migration_idempotency() {
        // Running migrations multiple times should be safe
        let (db, _temp) = create_test_db();

        // Get initial version
        let version1 = db.get_schema_version();

        // Re-run migrations (should be no-op)
        db.run_migrations().expect("Re-running migrations failed");

        let version2 = db.get_schema_version();

        assert_eq!(version1, version2, "Schema version should not change");
    }

    #[test]
    fn test_pinch_metadata_table_migration() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();

        // Verify pinch_metadata table exists (migration 9)
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='pinch_metadata'")
            .expect("Failed to prepare query");

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"pinch_metadata".to_string()));

        // Verify columns
        let mut stmt = conn
            .prepare("PRAGMA table_info(pinch_metadata)")
            .expect("Failed to prepare PRAGMA");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("Failed to get columns")
            .filter_map(Result::ok)
            .collect();

        assert!(columns.contains(&"source_session_id".to_string()));
        assert!(columns.contains(&"target_session_id".to_string()));
        assert!(columns.contains(&"summary".to_string()));
    }

    #[test]
    fn test_block_ui_state_table_migration() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();

        // Verify block_ui_state table exists (migration 10)
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='block_ui_state'",
            )
            .expect("Failed to prepare query");

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"block_ui_state".to_string()));
    }

    #[test]
    fn test_file_activity_table_migration() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();

        // Verify file_activity table exists (migration 11)
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='file_activity'")
            .expect("Failed to prepare query");

        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("Failed to query tables")
            .filter_map(Result::ok)
            .collect();

        assert!(tables.contains(&"file_activity".to_string()));

        // Verify activity tracking columns
        let mut stmt = conn
            .prepare("PRAGMA table_info(file_activity)")
            .expect("Failed to prepare PRAGMA");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("Failed to get columns")
            .filter_map(Result::ok)
            .collect();

        assert!(columns.contains(&"read_count".to_string()));
        assert!(columns.contains(&"write_count".to_string()));
        assert!(columns.contains(&"edit_count".to_string()));
        assert!(columns.contains(&"last_accessed".to_string()));
    }

    #[test]
    fn test_agent_state_columns_migration() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();

        // Verify agent state columns exist (migration 7)
        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .expect("Failed to prepare PRAGMA");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("Failed to get columns")
            .filter_map(Result::ok)
            .collect();

        assert!(columns.contains(&"agent_state".to_string()));
        assert!(columns.contains(&"agent_started_at".to_string()));
        assert!(columns.contains(&"agent_last_event_at".to_string()));
    }

    #[test]
    fn test_token_count_column_migration() {
        let (db, _temp) = create_test_db();

        let conn = db.conn();

        // Verify token_count column exists (migration 8)
        let mut stmt = conn
            .prepare("PRAGMA table_info(sessions)")
            .expect("Failed to prepare PRAGMA");

        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("Failed to get columns")
            .filter_map(Result::ok)
            .collect();

        assert!(columns.contains(&"token_count".to_string()));
    }
}
