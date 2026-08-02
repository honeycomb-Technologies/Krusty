//! Tests for database migrations
//!
//! These tests verify that:
//! - All migrations apply successfully
//! - Schema version is tracked correctly
//! - Migrations can be rolled back (conceptually - we don't actually rollback)
//! - Data survives through migrations

mod core;
mod feature_tables;
mod migrations;
mod runtime_tables;

use tempfile::TempDir;

use crate::storage::database::Database;

fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).expect("Failed to create database");
    (db, temp_dir)
}
