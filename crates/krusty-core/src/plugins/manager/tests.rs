use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::Sha256;
use tempfile::tempdir;
use tokio::fs;

#[tokio::test]
async fn installs_local_manifest_with_signature_verification() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 1
id = "demo.plugin"
name = "Demo Plugin"
version = "1.0.0"
publisher = "demo.publisher"
entry_component = "demo.wasm"

[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "demo-key"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect("add key");

    let installed = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect("install plugin");

    assert_eq!(installed.id, "demo.plugin");
    assert_eq!(installed.version, "1.0.0");
    assert!(installed.entry_component_path.exists());

    let plugins = manager
        .list_installed_plugins()
        .await
        .expect("list installed");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "demo.plugin");
}

#[tokio::test]
async fn rejects_non_allowlisted_publishers() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[11u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 1
id = "blocked.plugin"
name = "Blocked Plugin"
version = "1.0.0"
publisher = "blocked.publisher"
entry_component = "demo.wasm"

[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "blocked-key"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_trusted_key("blocked-key", &public_key_b64)
        .await
        .expect("add key");

    let err = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect_err("install should fail");
    assert!(
        err.to_string().contains("is not allowlisted"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_path_traversal_in_manifest_id() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[9u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 1
id = "../escape"
name = "Escape Plugin"
version = "1.0.0"
publisher = "demo.publisher"
entry_component = "demo.wasm"

[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "demo-key"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect("add key");

    let err = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect_err("install should fail");
    assert!(
        err.to_string().contains("cannot contain path separators"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_unsupported_manifest_version() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[12u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 2
id = "demo.plugin"
name = "Demo Plugin"
version = "1.0.0"
publisher = "demo.publisher"
entry_component = "demo.wasm"

[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "demo-key"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect("add key");

    let err = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect_err("install should fail");
    assert!(
        err.to_string().contains("unsupported manifest version"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_incompatible_krusty_version_bounds() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[13u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 1
id = "demo.plugin"
name = "Demo Plugin"
version = "1.0.0"
publisher = "demo.publisher"
entry_component = "demo.wasm"

[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "demo-key"

[compat]
krusty_min = "99.0.0"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect("add key");

    let err = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect_err("install should fail");
    assert!(
        err.to_string().contains("requires krusty >="),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_local_release_path_traversal() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let manifest_dir = workspace.join("manifest");
    fs::create_dir_all(&manifest_dir)
        .await
        .expect("create manifest dir");

    let artifact_bytes = b"fake-wasm-component".to_vec();
    let artifact_path = manifest_dir.join("demo.wasm");
    fs::write(&artifact_path, &artifact_bytes)
        .await
        .expect("write artifact");

    let signing_key = SigningKey::from_bytes(&[14u8; 32]);
    let signature = signing_key.sign(&artifact_bytes);
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let signature_b64 = BASE64.encode(signature.to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(
        &manifest_path,
        format!(
            r#"
manifest_version = 1
id = "demo.plugin"
name = "Demo Plugin"
version = "1.0.0"
publisher = "demo.publisher"
entry_component = "demo.wasm"

[release]
url = "../demo.wasm"
sha256 = "{sha}"
signature = "{signature_b64}"
signing_key_id = "demo-key"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect("add key");

    let err = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect_err("install should fail");
    assert!(
        err.to_string().contains("invalid local release path"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_invalid_trusted_key_material() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let err = manager
        .add_trusted_key("bad-key", "not-base64")
        .await
        .expect_err("invalid key should be rejected");
    assert!(
        err.to_string().contains("invalid trusted key encoding"),
        "unexpected error: {}",
        err
    );
}
