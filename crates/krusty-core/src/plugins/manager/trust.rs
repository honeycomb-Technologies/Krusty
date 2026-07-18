use anyhow::{bail, Result};

use super::super::signing::validate_public_key_base64;
use super::storage::{load_trust_policy, save_trust_policy};
use super::PluginManager;

impl PluginManager {
    pub async fn add_allowed_publisher(&self, publisher: &str) -> Result<()> {
        let _guard = self.acquire_mutation().await?;
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

    #[deprecated(
        note = "publisher-less signing keys are unusable; call add_trusted_key_for_publisher"
    )]
    pub async fn add_trusted_key(&self, _key_id: &str, public_key_b64: &str) -> Result<()> {
        validate_public_key_base64(public_key_b64)?;
        bail!(
            "trusted signing keys require an explicit publisher binding; use add_trusted_key_for_publisher(publisher, key_id, public_key_b64)"
        )
    }

    pub async fn add_trusted_key_for_publisher(
        &self,
        publisher: &str,
        key_id: &str,
        public_key_b64: &str,
    ) -> Result<()> {
        validate_public_key_base64(public_key_b64)?;
        validate_binding_identity(publisher, key_id)?;
        let _guard = self.acquire_mutation().await?;

        let mut trust = load_trust_policy(self).await?;
        reject_key_id_reassignment(&trust, key_id, public_key_b64)?;
        trust
            .keys
            .insert(key_id.to_string(), public_key_b64.to_string());
        let keys = trust
            .publisher_keys
            .entry(publisher.to_string())
            .or_default();
        if !keys.iter().any(|existing| existing == key_id) {
            keys.push(key_id.to_string());
            keys.sort();
        }
        save_trust_policy(self, &trust).await
    }

    /// Explicitly binds key material already present in a legacy trust file to
    /// one publisher. No publisher is inferred from the allowlist.
    pub async fn bind_existing_trusted_key_to_publisher(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<()> {
        validate_binding_identity(publisher, key_id)?;
        let _guard = self.acquire_mutation().await?;
        let mut trust = load_trust_policy(self).await?;
        if !trust.keys.contains_key(key_id) {
            bail!(
                "trusted key '{}' does not exist; add it with an explicit publisher binding first",
                key_id
            );
        }
        let keys = trust
            .publisher_keys
            .entry(publisher.to_string())
            .or_default();
        if !keys.iter().any(|existing| existing == key_id) {
            keys.push(key_id.to_string());
            keys.sort();
            save_trust_policy(self, &trust).await?;
        }
        Ok(())
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

    pub(super) async fn verify_publisher_key_binding(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<()> {
        let trust = load_trust_policy(self).await?;
        if trust
            .publisher_keys
            .get(publisher)
            .map(|keys| keys.iter().any(|candidate| candidate == key_id))
            .unwrap_or(false)
        {
            return Ok(());
        }
        if trust.keys.contains_key(key_id) {
            bail!(
                "legacy trusted key '{}' is not bound to publisher '{}'; explicitly rebind the existing key with bind_existing_trusted_key_to_publisher or re-add it with /plugins add-key <key-id> <public-key-base64> <publisher>",
                key_id,
                publisher
            );
        }
        bail!(
            "signing key '{}' is not bound to publisher '{}'; register the publisher-key binding before install",
            key_id,
            publisher
        )
    }
}

fn validate_binding_identity(publisher: &str, key_id: &str) -> Result<()> {
    if publisher.trim().is_empty() {
        bail!("publisher cannot be empty when binding a signing key");
    }
    if key_id.trim().is_empty() {
        bail!("signing key id cannot be empty when binding a signing key");
    }
    Ok(())
}

fn reject_key_id_reassignment(
    trust: &crate::plugins::PluginTrustPolicy,
    key_id: &str,
    public_key_b64: &str,
) -> Result<()> {
    if trust
        .keys
        .get(key_id)
        .map(|existing| existing != public_key_b64)
        .unwrap_or(false)
    {
        bail!(
            "trusted key id '{}' is already registered with different key material; use a new key id for rotation",
            key_id
        );
    }
    Ok(())
}
