use anyhow::{bail, Result};

use super::super::signing::validate_public_key_base64;
use super::storage::{load_trust_policy, save_trust_policy};
use super::PluginManager;

impl PluginManager {
    pub async fn add_allowed_publisher(&self, publisher: &str) -> Result<()> {
        let mut trust = load_trust_policy(self).await?;
        if !trust
            .allowed_publishers
            .iter()
            .any(|existing| existing == publisher)
        {
            trust.allowed_publishers.push(publisher.to_string());
            trust.allowed_publishers.sort();
            save_trust_policy(self, &trust).await?;
        }
        Ok(())
    }

    pub async fn add_trusted_key(&self, key_id: &str, public_key_b64: &str) -> Result<()> {
        validate_public_key_base64(public_key_b64)?;

        let mut trust = load_trust_policy(self).await?;
        trust
            .keys
            .insert(key_id.to_string(), public_key_b64.to_string());
        save_trust_policy(self, &trust).await
    }

    pub(super) async fn verify_publisher_allowed(&self, publisher: &str) -> Result<()> {
        let trust = load_trust_policy(self).await?;
        if trust
            .allowed_publishers
            .iter()
            .any(|allowed| allowed == publisher)
        {
            return Ok(());
        }

        bail!(
            "publisher '{}' is not allowlisted. Add it via plugin trust configuration first",
            publisher
        )
    }
}
