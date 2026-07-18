use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use super::{PluginManifestV1, PluginRelease};

const RELEASE_ENVELOPE_DOMAIN: &[u8] = b"krusty-plugin-release-envelope-v1\0";
pub(crate) const RELEASE_SIGNATURE_SCHEME: &str = "manifest-envelope-v1";

pub(crate) fn validate_release_signature_scheme(release: &PluginRelease) -> Result<()> {
    match release.signature_scheme.as_deref() {
        Some(RELEASE_SIGNATURE_SCHEME) => Ok(()),
        Some(other) => bail!(
            "unsupported release.signature_scheme '{}'; expected '{}'",
            other,
            RELEASE_SIGNATURE_SCHEME
        ),
        None => bail!(
            "release.signature_scheme is required for signed standalone installs; legacy artifact-only signatures cannot be inferred safely. Set signature_scheme = \"{}\" and re-sign the manifest",
            RELEASE_SIGNATURE_SCHEME
        ),
    }
}

/// Deterministic message signed by a plugin publisher.
///
/// The compact JSON includes the complete manifest (identity, runtime,
/// component paths, permissions, compatibility, release URL, digest, and key
/// id) with only the signature value blanked. The domain prefix prevents the
/// same signature from being reused for a different protocol.
pub fn plugin_release_signing_payload(manifest: &PluginManifestV1) -> Result<Vec<u8>> {
    let mut unsigned = manifest.clone();
    let release = unsigned
        .release
        .as_mut()
        .context("manifest release metadata is required for signing")?;
    validate_release_signature_scheme(release)?;
    release.signature.clear();

    let json = serde_json::to_vec(&unsigned).context("failed to encode plugin release envelope")?;
    let mut payload = Vec::with_capacity(RELEASE_ENVELOPE_DOMAIN.len() + json.len());
    payload.extend_from_slice(RELEASE_ENVELOPE_DOMAIN);
    payload.extend_from_slice(&json);
    Ok(payload)
}

pub fn validate_public_key_base64(public_key_b64: &str) -> Result<()> {
    decode_verifying_key(public_key_b64).map(|_| ())
}

pub fn verify_artifact_signature(
    signed_payload: &[u8],
    signature_b64: &str,
    public_key_b64: &str,
) -> Result<()> {
    let signature_raw = BASE64
        .decode(signature_b64)
        .context("invalid signature encoding (expected base64)")?;
    let signature = Signature::from_slice(&signature_raw)
        .map_err(|e| anyhow!("invalid signature bytes: {}", e))?;

    let verifying_key = decode_verifying_key(public_key_b64)?;

    verifying_key
        .verify(signed_payload, &signature)
        .map_err(|e| anyhow!("signature verification failed: {}", e))
}

fn decode_verifying_key(public_key_b64: &str) -> Result<VerifyingKey> {
    let key_raw = BASE64
        .decode(public_key_b64)
        .context("invalid trusted key encoding (expected base64)")?;
    let key_raw: [u8; 32] = key_raw
        .try_into()
        .map_err(|_| anyhow!("invalid trusted key length (expected 32 bytes)"))?;

    VerifyingKey::from_bytes(&key_raw).map_err(|e| anyhow!("invalid ed25519 public key: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_value() -> serde_json::Value {
        serde_json::json!({
            "manifest_version": 1,
            "id": "signed.plugin",
            "name": "Signed Plugin",
            "version": "1.0.0",
            "publisher": "signed.publisher",
            "runtime": "wasm",
            "entry_component": "plugin.wasm",
            "requested_permissions": {
                "process": false
            },
            "release": {
                "url": "https://plugins.example/plugin.wasm",
                "sha256": "00".repeat(32),
                "signature": "signature",
                "signing_key_id": "release-key",
                "signature_scheme": "manifest-envelope-v1"
            },
            "compat": {
                "krusty_min": "0.7.0"
            }
        })
    }

    #[test]
    fn release_envelope_rejects_unsigned_unknown_fields() {
        let mut cases = Vec::new();

        let mut top_level = manifest_value();
        top_level["future_executable_component"] = serde_json::json!("run.js");
        cases.push(top_level);

        let mut release = manifest_value();
        release["release"]["unsigned_mirror"] =
            serde_json::json!("https://evil.example/plugin.wasm");
        cases.push(release);

        let mut permissions = manifest_value();
        permissions["requested_permissions"]["future_host_authority"] = serde_json::json!(true);
        cases.push(permissions);

        let mut compat = manifest_value();
        compat["compat"]["future_runtime"] = serde_json::json!("unsafe");
        cases.push(compat);

        for value in cases {
            let error = serde_json::from_value::<PluginManifestV1>(value)
                .expect_err("unknown signed-envelope field must be rejected");
            assert!(error.to_string().contains("unknown field"));
        }
    }

    #[test]
    fn release_envelope_requires_an_explicit_signature_scheme() {
        let mut value = manifest_value();
        value["release"]
            .as_object_mut()
            .expect("release object")
            .remove("signature_scheme");
        let manifest: PluginManifestV1 =
            serde_json::from_value(value).expect("parse legacy manifest");

        let error = plugin_release_signing_payload(&manifest)
            .expect_err("legacy signature semantics must not be inferred");

        assert!(error
            .to_string()
            .contains("legacy artifact-only signatures"));
        assert!(error.to_string().contains(RELEASE_SIGNATURE_SCHEME));
    }

    #[test]
    fn release_envelope_rejects_unknown_signature_schemes() {
        let mut value = manifest_value();
        value["release"]["signature_scheme"] = serde_json::json!("future-scheme-v9");
        let manifest: PluginManifestV1 =
            serde_json::from_value(value).expect("parse future manifest");

        let error = plugin_release_signing_payload(&manifest)
            .expect_err("unknown signature scheme must fail closed");

        assert!(error
            .to_string()
            .contains("unsupported release.signature_scheme"));
    }

    #[test]
    fn release_artifact_kind_is_authenticated_without_changing_legacy_default_payloads() {
        let default_manifest: PluginManifestV1 =
            serde_json::from_value(manifest_value()).expect("parse default manifest");
        let default_payload =
            plugin_release_signing_payload(&default_manifest).expect("default signing payload");
        let default_json: serde_json::Value =
            serde_json::from_slice(&default_payload[RELEASE_ENVELOPE_DOMAIN.len()..])
                .expect("parse default envelope");
        assert!(default_json["release"].get("artifact_kind").is_none());

        let mut bundle_value = manifest_value();
        bundle_value
            .as_object_mut()
            .expect("manifest object")
            .remove("entry_component");
        bundle_value["skills"] = serde_json::json!(["skills/demo/SKILL.md"]);
        bundle_value["release"]["artifact_kind"] = serde_json::json!("zip-bundle");
        let bundle_manifest: PluginManifestV1 =
            serde_json::from_value(bundle_value).expect("parse bundle manifest");
        let bundle_payload =
            plugin_release_signing_payload(&bundle_manifest).expect("bundle signing payload");
        let bundle_json: serde_json::Value =
            serde_json::from_slice(&bundle_payload[RELEASE_ENVELOPE_DOMAIN.len()..])
                .expect("parse bundle envelope");
        assert_eq!(
            bundle_json["release"]["artifact_kind"],
            serde_json::json!("zip-bundle")
        );
    }
}
