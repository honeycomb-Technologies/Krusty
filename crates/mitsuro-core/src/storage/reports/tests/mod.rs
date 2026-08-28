use rusqlite::params;
use tempfile::TempDir;

use super::disk::slugify;
use super::{CreateReportInput, ReportScope, ReportStore};
use crate::storage::Database;

fn create_store() -> (ReportStore, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let db = Database::new(&tmp.path().join("reports.db")).expect("db");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params!["sess-1", "Report Test", now, now],
        )
        .expect("seed session");
    (ReportStore::new(db), tmp)
}

fn create_store_with_users() -> (ReportStore, TempDir) {
    let (store, tmp) = create_store();
    let now = chrono::Utc::now().to_rfc3339();
    store
        .db
        .conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            params!["user-a", "user-a@example.com", "free"],
        )
        .expect("seed user a");
    store
        .db
        .conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            params!["user-b", "user-b@example.com", "free"],
        )
        .expect("seed user b");
    store
        .db
        .conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["sess-a", "User A Session", now, now, "user-a"],
        )
        .expect("seed owned session a");
    store
        .db
        .conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["sess-b", "User B Session", now, now, "user-b"],
        )
        .expect("seed owned session b");
    (store, tmp)
}

mod disk;
mod ownership;
mod store;
