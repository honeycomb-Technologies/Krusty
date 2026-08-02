use anyhow::Result;

use crate::tools::git_identity::GitIdentity;

use super::core::Preferences;

impl Preferences {
    pub fn get_theme(&self) -> String {
        self.get("theme").unwrap_or_else(|| "mitsuro".to_string())
    }

    pub fn set_theme(&self, theme: &str) -> Result<()> {
        self.set("theme", theme)
    }

    pub fn get_active_plugin(&self) -> Option<String> {
        self.get("active_plugin")
    }

    pub fn set_active_plugin(&self, plugin_id: &str) -> Result<()> {
        self.set("active_plugin", plugin_id)
    }

    pub fn get_git_identity(&self) -> GitIdentity {
        self.get("git_identity")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn set_git_identity(&self, identity: &GitIdentity) -> Result<()> {
        let json = serde_json::to_string(identity)?;
        self.set("git_identity", &json)
    }
}
