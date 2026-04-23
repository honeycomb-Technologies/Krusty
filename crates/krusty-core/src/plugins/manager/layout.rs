use anyhow::Result;
use tokio::fs;

use super::storage::{load_lockfile, load_sources, load_trust_policy};
use super::PluginManager;

impl PluginManager {
    /// Ensure required plugin directories and config files exist.
    pub async fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.installed_root()).await?;
        fs::create_dir_all(self.active_root()).await?;
        fs::create_dir_all(self.state_root()).await?;
        fs::create_dir_all(self.index_root()).await?;
        fs::create_dir_all(self.trust_root()).await?;

        load_lockfile(self).await?;
        load_trust_policy(self).await?;
        load_sources(self).await?;

        Ok(())
    }
}
