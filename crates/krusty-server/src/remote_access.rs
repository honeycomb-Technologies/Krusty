use anyhow::Result;
use serde::Serialize;
use uuid::Uuid;

use krusty_core::storage::{Database, Preferences};

const REMOTE_ACCESS_ENABLED_KEY: &str = "server_remote_access_enabled";
const REMOTE_ACCESS_TOKEN_KEY: &str = "server_remote_access_token";

#[derive(Debug, Clone, Serialize)]
pub struct RemoteAccessConfig {
    pub enabled: bool,
    pub token: String,
}

impl RemoteAccessConfig {
    pub fn load_or_create(db_path: &std::path::Path) -> Result<Self> {
        let db = Database::new(db_path)?;
        let preferences = Preferences::new(db);
        let enabled = preferences
            .get(REMOTE_ACCESS_ENABLED_KEY)
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let token = match preferences.get(REMOTE_ACCESS_TOKEN_KEY) {
            Some(token) if !token.trim().is_empty() => token,
            _ => {
                let token = generate_token();
                preferences.set(REMOTE_ACCESS_TOKEN_KEY, &token)?;
                token
            }
        };

        if preferences.get(REMOTE_ACCESS_ENABLED_KEY).is_none() {
            preferences.set(
                REMOTE_ACCESS_ENABLED_KEY,
                if enabled { "true" } else { "false" },
            )?;
        }

        Ok(Self { enabled, token })
    }

    pub fn persist(&self, db_path: &std::path::Path) -> Result<()> {
        let db = Database::new(db_path)?;
        let preferences = Preferences::new(db);
        preferences.set(
            REMOTE_ACCESS_ENABLED_KEY,
            if self.enabled { "true" } else { "false" },
        )?;
        preferences.set(REMOTE_ACCESS_TOKEN_KEY, &self.token)?;
        Ok(())
    }

    pub fn rotate(&mut self, db_path: &std::path::Path) -> Result<()> {
        self.token = generate_token();
        self.persist(db_path)
    }
}

fn generate_token() -> String {
    format!("kr_remote_{}", Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::RemoteAccessConfig;

    #[test]
    fn load_or_create_persists_and_reuses_token() {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-remote-access-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should create");
        let db_path = temp_dir.join("test.db");

        let first = RemoteAccessConfig::load_or_create(&db_path).expect("load should succeed");
        let second = RemoteAccessConfig::load_or_create(&db_path).expect("reload should succeed");

        assert_eq!(first.token, second.token);
        assert!(!second.enabled);
    }
}
