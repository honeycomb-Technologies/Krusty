use super::*;
use crate::plugins::PluginManifestV1;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use tokio::fs;

fn entry_manifest(runtime: &str, process: bool) -> PluginManifestV1 {
    toml::from_str(&format!(
        r#"
manifest_version = 1
id = "runtime.permission"
name = "Runtime Permission"
version = "1.0.0"
publisher = "test.publisher"
runtime = "{runtime}"
entry_component = "entry.bin"

[requested_permissions]
process = {process}

[release]
url = "https://example.com/entry.bin"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
signature = "test-signature"
signing_key_id = "test-key"
signature_scheme = "manifest-envelope-v1"
"#
    ))
    .expect("parse runtime manifest")
}

#[test]
fn native_and_js_entries_require_process_declaration_for_all_install_paths() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));

    for runtime in ["native", "js"] {
        for require_release in [false, true] {
            let error = manager
                .validate_manifest(&entry_manifest(runtime, false), require_release)
                .expect_err("unsafe entry runtime must declare process permission");
            assert!(
                error.to_string().contains(&format!(
                    "{runtime} entry_component requires requested_permissions.process = true"
                )),
                "unexpected validation error: {error:#}"
            );
        }
    }
}

#[test]
fn wasm_entry_does_not_require_process_declaration() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));

    manager
        .validate_manifest(&entry_manifest("wasm", false), false)
        .expect("package WASM entry should remain allowed");
    manager
        .validate_manifest(&entry_manifest("wasm", false), true)
        .expect("signed WASM entry should remain allowed");
}

#[test]
fn signed_standalone_manifest_requires_an_explicit_signature_scheme() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    let mut manifest = entry_manifest("wasm", false);
    manifest
        .release
        .as_mut()
        .expect("release metadata")
        .signature_scheme = None;

    let error = manager
        .validate_manifest(&manifest, true)
        .expect_err("legacy signature semantics must not be inferred");

    assert!(error
        .to_string()
        .contains("legacy artifact-only signatures"));
    assert!(error.to_string().contains("manifest-envelope-v1"));
}

#[allow(deprecated)]
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
    let public_key_b64 = BASE64.encode(signing_key.verifying_key().to_bytes());
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));

    let manifest_path = manifest_dir.join("plugin.toml");
    let unsigned_manifest = format!(
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
signature = "SIGNATURE_PLACEHOLDER"
signing_key_id = "demo-key"
signature_scheme = "manifest-envelope-v1"
"#
    );
    let parsed_manifest: PluginManifestV1 = toml::from_str(&unsigned_manifest).unwrap();
    let signature = signing_key.sign(
        &crate::plugins::plugin_release_signing_payload(&parsed_manifest)
            .expect("release signing payload"),
    );
    let signature_b64 = BASE64.encode(signature.to_bytes());
    fs::write(
        &manifest_path,
        unsigned_manifest.replace("SIGNATURE_PLACEHOLDER", &signature_b64),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_allowed_publisher("demo.publisher")
        .await
        .expect("allow publisher");
    let unbound_error = manager
        .add_trusted_key("demo-key", &public_key_b64)
        .await
        .expect_err("publisher-less trusted keys must fail clearly");
    assert!(unbound_error
        .to_string()
        .contains("require an explicit publisher binding"));
    manager
        .add_trusted_key_for_publisher("demo.publisher", "demo-key", &public_key_b64)
        .await
        .expect("bind publisher key");

    let installed = manager
        .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
        .await
        .expect("install plugin");

    assert_eq!(installed.id, "demo.plugin");
    assert_eq!(installed.version, "1.0.0");
    let canonical_manifest = fs::canonicalize(&manifest_path)
        .await
        .expect("canonical manifest path");
    assert_eq!(
        installed.source.as_deref(),
        canonical_manifest.to_str(),
        "signed installs must retain a stable canonical provenance source"
    );
    assert!(installed
        .entry_component_path
        .as_ref()
        .expect("entry component")
        .exists());

    let plugins = manager
        .list_installed_plugins()
        .await
        .expect("list installed");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "demo.plugin");
}

#[tokio::test]
async fn signed_artifact_cannot_be_replayed_under_modified_manifest_metadata() {
    let temp = tempdir().expect("tempdir");
    let manifest_dir = temp.path().join("manifest");
    fs::create_dir_all(&manifest_dir).await.unwrap();
    let artifact_bytes = b"signed-code".to_vec();
    fs::write(manifest_dir.join("demo.wasm"), &artifact_bytes)
        .await
        .unwrap();
    let sha = format!("{:x}", Sha256::digest(&artifact_bytes));
    let signing_key = SigningKey::from_bytes(&[31u8; 32]);
    let unsigned = format!(
        r#"
manifest_version = 1
id = "authentic.plugin"
name = "Authentic Plugin"
version = "1.0.0"
publisher = "authentic.publisher"
entry_component = "demo.wasm"
[release]
url = "demo.wasm"
sha256 = "{sha}"
signature = "SIGNATURE_PLACEHOLDER"
signing_key_id = "authentic-key"
signature_scheme = "manifest-envelope-v1"
"#
    );
    let manifest: PluginManifestV1 = toml::from_str(&unsigned).unwrap();
    let signature = signing_key.sign(
        &crate::plugins::plugin_release_signing_payload(&manifest)
            .expect("release signing payload"),
    );
    let signed = unsigned.replace(
        "SIGNATURE_PLACEHOLDER",
        &BASE64.encode(signature.to_bytes()),
    );
    // Replay the authentic artifact/signature while escalating the declared
    // identity and permission envelope.
    let tampered = signed
        .replace("authentic.plugin", "replayed.plugin")
        .replace(
            "entry_component = \"demo.wasm\"",
            "entry_component = \"demo.wasm\"\n[requested_permissions]\nprocess = true",
        );
    let manifest_path = manifest_dir.join("plugin.toml");
    fs::write(&manifest_path, tampered).await.unwrap();

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.unwrap();
    manager
        .add_allowed_publisher("authentic.publisher")
        .await
        .unwrap();
    manager
        .add_trusted_key_for_publisher(
            "authentic.publisher",
            "authentic-key",
            &BASE64.encode(signing_key.verifying_key().to_bytes()),
        )
        .await
        .unwrap();

    let error = manager
        .install_from_manifest_ref(manifest_path.to_str().unwrap())
        .await
        .expect_err("tampered manifest metadata must invalidate the signature");
    assert!(error.to_string().contains("signature verification failed"));
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
signature_scheme = "manifest-envelope-v1"
"#
        ),
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_trusted_key_for_publisher("blocked.publisher", "blocked-key", &public_key_b64)
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
signature_scheme = "manifest-envelope-v1"
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
        .add_trusted_key_for_publisher("demo.publisher", "demo-key", &public_key_b64)
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
signature_scheme = "manifest-envelope-v1"
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
        .add_trusted_key_for_publisher("demo.publisher", "demo-key", &public_key_b64)
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
signature_scheme = "manifest-envelope-v1"

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
        .add_trusted_key_for_publisher("demo.publisher", "demo-key", &public_key_b64)
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
signature_scheme = "manifest-envelope-v1"
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
        .add_trusted_key_for_publisher("demo.publisher", "demo-key", &public_key_b64)
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
        .add_trusted_key_for_publisher("demo.publisher", "bad-key", "not-base64")
        .await
        .expect_err("invalid key should be rejected");
    assert!(
        err.to_string().contains("invalid trusted key encoding"),
        "unexpected error: {}",
        err
    );
}

#[tokio::test]
async fn rejects_silent_trusted_key_id_reassignment() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let first = SigningKey::from_bytes(&[21u8; 32]);
    let second = SigningKey::from_bytes(&[22u8; 32]);
    manager
        .add_trusted_key_for_publisher(
            "demo.publisher",
            "release-key",
            &BASE64.encode(first.verifying_key().to_bytes()),
        )
        .await
        .expect("add first key");
    let error = manager
        .add_trusted_key_for_publisher(
            "demo.publisher",
            "release-key",
            &BASE64.encode(second.verifying_key().to_bytes()),
        )
        .await
        .expect_err("key id reassignment must fail");
    assert!(error.to_string().contains("different key material"));
}

#[tokio::test]
async fn explicitly_binds_an_existing_legacy_trusted_key() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let key = SigningKey::from_bytes(&[23u8; 32]);
    let key_b64 = BASE64.encode(key.verifying_key().to_bytes());
    let mut trust = super::storage::load_trust_policy(&manager)
        .await
        .expect("load trust policy");
    trust.keys.insert("legacy-key".to_string(), key_b64);
    super::storage::save_trust_policy(&manager, &trust)
        .await
        .expect("save legacy trust policy");

    manager
        .bind_existing_trusted_key_to_publisher("legacy.publisher", "legacy-key")
        .await
        .expect("explicitly bind legacy key");

    let trust = super::storage::load_trust_policy(&manager)
        .await
        .expect("reload trust policy");
    assert_eq!(
        trust.publisher_keys.get("legacy.publisher"),
        Some(&vec!["legacy-key".to_string()])
    );
}

#[cfg(unix)]
#[tokio::test]
async fn managed_snapshot_cleanup_removes_symlink_entry_not_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.unwrap();
    let victim = manager.managed_root().join("victim");
    fs::create_dir_all(&victim).await.unwrap();
    fs::write(victim.join("keep.txt"), "keep").await.unwrap();
    let hostile = manager.managed_root().join("hostile-link");
    symlink(&victim, &hostile).unwrap();

    super::transaction::remove_manager_owned_root(&manager, &hostile)
        .await
        .unwrap();

    assert!(!hostile.exists());
    assert_eq!(
        fs::read_to_string(victim.join("keep.txt")).await.unwrap(),
        "keep"
    );
}

#[tokio::test]
async fn installs_local_package_declared_by_package_json() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let package_dir = workspace.join("native-package");
    let dist_dir = package_dir.join("dist/linux-x64");
    fs::create_dir_all(&dist_dir).await.expect("create dist");
    fs::write(dist_dir.join("libdemo_plugin.so"), b"not-a-real-library")
        .await
        .expect("write native artifact");
    fs::write(
        package_dir.join("package.json"),
        r#"{
  "name": "@krusty/demo-plugin",
  "version": "1.0.0",
  "krusty": { "plugins": ["./plugin.toml"] }
}"#,
    )
    .await
    .expect("write package json");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.native"
name = "Demo Native"
version = "1.0.0"
publisher = "demo.publisher"
runtime = "native"
entry_component = "dist/linux-x64/libdemo_plugin.so"
render_capabilities = ["text"]

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write plugin manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let installed = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].id, "demo.native");
    assert_eq!(installed[0].runtime, crate::plugins::PluginRuntime::Native);
    assert_ne!(
        installed[0].install_path,
        fs::canonicalize(&package_dir).await.unwrap()
    );
    let canonical_installed_root = fs::canonicalize(manager.installed_root())
        .await
        .expect("canonical installed root");
    assert!(installed[0]
        .install_path
        .starts_with(canonical_installed_root));
    assert!(installed[0]
        .entry_component_path
        .as_ref()
        .expect("entry component")
        .ends_with("dist/linux-x64/libdemo_plugin.so"));

    let plugins = manager
        .list_installed_plugins()
        .await
        .expect("list installed");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "demo.native");
}

#[tokio::test]
async fn local_package_build_script_requires_explicit_consent() {
    if which::which("npm").is_err() {
        eprintln!("skipping npm build-script test because npm is unavailable");
        return;
    }

    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let package_dir = workspace.join("buildable-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package dir");
    fs::write(
        package_dir.join("package.json"),
        r#"{
  "name": "@krusty/buildable-plugin",
  "version": "1.0.0",
  "scripts": {
    "build": "mkdir -p dist/linux-x64 && printf plugin > dist/linux-x64/libdemo_plugin.so"
  },
  "krusty": { "plugins": ["./plugin.toml"] }
}"#,
    )
    .await
    .expect("write package json");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.buildable"
name = "Buildable Native"
version = "1.0.0"
publisher = "demo.publisher"
runtime = "native"
entry_component = "dist/linux-x64/libdemo_plugin.so"
render_capabilities = ["text"]

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write plugin manifest");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let error = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect_err("scripts must be denied by default");
    assert!(error
        .to_string()
        .contains("scripts are disabled by default"));
    assert!(!package_dir
        .join("dist/linux-x64/libdemo_plugin.so")
        .exists());

    let installed = manager
        .install_from_ref_with_options(
            package_dir.to_str().expect("package path utf8"),
            crate::plugins::PluginInstallOptions {
                allow_package_scripts: true,
                pinned: None,
            },
        )
        .await
        .expect("install package with explicit script consent");

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].id, "demo.buildable");
    assert!(installed[0]
        .entry_component_path
        .as_ref()
        .expect("entry component")
        .exists());
    assert!(installed[0].package_scripts_allowed);
}

#[tokio::test]
async fn lists_builtin_and_configured_catalog_plugins() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let catalog_path = workspace.join("catalog.json");
    fs::write(
        &catalog_path,
        r#"{
  "version": 1,
  "plugins": [
    {
      "id": "catalog.demo",
      "name": "Catalog Demo",
      "version": "1.0.0",
      "publisher": "catalog.publisher",
      "package": "npm:@krusty/catalog-demo",
      "runtime": "wasm",
      "description": "Demo catalog entry",
      "tags": ["demo"],
      "official": true
    }
  ]
}"#,
    )
    .await
    .expect("write catalog");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .add_source(
            Some("local"),
            catalog_path.to_str().expect("catalog path utf8"),
        )
        .await
        .expect("add source");

    let entries = manager.list_catalog_plugins().await.expect("list catalog");

    assert!(entries.iter().any(|entry| entry.id == "native-rust-demo"));
    let configured = entries
        .iter()
        .find(|entry| entry.id == "catalog.demo")
        .expect("configured catalog entry");
    assert!(!configured.official);
}

#[tokio::test]
async fn installs_local_js_package_declared_by_package_json() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path();
    let package_dir = workspace.join("js-package");
    fs::create_dir_all(package_dir.join("src"))
        .await
        .expect("create package dirs");
    fs::write(
        package_dir.join("package.json"),
        r#"{
  "name": "@krusty/js-plugin",
  "version": "1.0.0",
  "krusty": { "plugins": ["./plugin.toml"] }
}"#,
    )
    .await
    .expect("write package json");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.js"
name = "Demo JS"
version = "1.0.0"
publisher = "demo.publisher"
runtime = "js"
entry_component = "src/index.ts"
render_capabilities = ["text"]

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write plugin manifest");
    fs::write(
        package_dir.join("src/index.ts"),
        "krusty.registerPlugin({ renderText() { return ['hi']; } });",
    )
    .await
    .expect("write plugin entry");

    let manager = PluginManager::new(reqwest::Client::new(), workspace.join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let installed = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].id, "demo.js");
    assert_eq!(installed[0].runtime, crate::plugins::PluginRuntime::Js);
}

#[tokio::test]
async fn installs_bundle_only_package_and_resolves_every_component() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("bundle");
    for directory in ["skills/demo", "extensions", "mcp", "hooks", "assets"] {
        fs::create_dir_all(package_dir.join(directory))
            .await
            .expect("create component directory");
    }
    fs::write(package_dir.join("skills/demo/SKILL.md"), "# Demo")
        .await
        .expect("write skill");
    fs::write(package_dir.join("extensions/demo.ts"), "export {}")
        .await
        .expect("write extension");
    fs::write(package_dir.join("mcp/servers.json"), "{}")
        .await
        .expect("write mcp config");
    fs::write(package_dir.join("hooks/hooks.json"), "{}")
        .await
        .expect("write hooks");
    fs::write(package_dir.join("assets/icon.txt"), "icon")
        .await
        .expect("write asset");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.bundle"
name = "Demo Bundle"
version = "1.0.0"
publisher = "demo.publisher"
skills = ["skills/demo"]
agent_extensions = ["extensions/demo.ts"]
mcp_servers = "mcp/servers.json"
hooks = ["hooks/hooks.json"]
assets = "assets"

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let installed = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install bundle");

    let plugin = &installed[0];
    assert!(plugin.entry_component_path.is_none());
    assert_eq!(plugin.skill_paths.len(), 1);
    assert_eq!(plugin.agent_extension_paths.len(), 1);
    assert!(plugin.mcp_servers_path.is_some());
    assert_eq!(plugin.hook_paths.len(), 1);
    assert!(plugin.assets_path.is_some());
    assert!(plugin.has_agent_components());
}

#[tokio::test]
async fn rejects_executable_entries_in_declarative_hooks() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("executable-hook");
    fs::create_dir_all(package_dir.join("hooks"))
        .await
        .expect("create hooks directory");
    fs::write(package_dir.join("hooks/pre.ts"), "export {}")
        .await
        .expect("write executable hook");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.executable-hook"
name = "Executable Hook Demo"
version = "1.0.0"
publisher = "demo.publisher"
hooks = ["hooks/pre.ts"]

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let error = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect_err("executable hook entry should be rejected");

    assert!(error.to_string().contains("declarative .json or .toml"));
    assert!(error.to_string().contains("agent_extensions"));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinks_in_local_package_snapshots() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("symlink-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(temp.path().join("outside.ts"), "outside")
        .await
        .expect("write outside file");
    symlink(temp.path().join("outside.ts"), package_dir.join("entry.ts")).expect("create symlink");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.symlink"
name = "Symlink Demo"
version = "1.0.0"
publisher = "demo.publisher"
runtime = "js"
entry_component = "entry.ts"

[requested_permissions]
process = true
"#,
    )
    .await
    .expect("write manifest");

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let error = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect_err("symlink package should fail");
    assert!(error.to_string().contains("may not contain symlinks"));
}

#[tokio::test]
async fn grants_are_explicit_subset_bound_and_fail_closed_after_request_change() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("permissions-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("entry.ts"), "export {}")
        .await
        .expect("write entry");
    write_permission_manifest(&package_dir, "1.0.0", false).await;

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");

    let initial = manager
        .permission_status("demo.permissions")
        .await
        .expect("permission status");
    assert!(!initial.grant_is_current);
    assert!(manager
        .ensure_plugin_permission("demo.permissions", crate::plugins::PluginPermission::FsRead,)
        .await
        .is_err());

    manager
        .grant_plugin_permissions(
            "demo.permissions",
            crate::plugins::PluginPermissionSet {
                fs_read: true,
                ..Default::default()
            },
        )
        .await
        .expect("grant subset");
    manager
        .ensure_plugin_permission("demo.permissions", crate::plugins::PluginPermission::FsRead)
        .await
        .expect("fs read granted");
    assert!(manager
        .ensure_plugin_permission(
            "demo.permissions",
            crate::plugins::PluginPermission::Network,
        )
        .await
        .is_err());

    write_permission_manifest(&package_dir, "2.0.0", true).await;
    let update = manager
        .update_plugin("demo.permissions", true)
        .await
        .expect("update package");
    assert_eq!(update.updated.len(), 1);
    let after_update = manager
        .permission_status("demo.permissions")
        .await
        .expect("permission status after update");
    assert!(!after_update.grant_is_current);
    assert!(manager
        .ensure_plugin_permission("demo.permissions", crate::plugins::PluginPermission::FsRead,)
        .await
        .is_err());
}

#[tokio::test]
async fn grants_are_fail_closed_after_version_change_with_same_permissions() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("permission-version-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("entry.ts"), "export {}")
        .await
        .expect("write entry");
    write_permission_manifest(&package_dir, "1.0.0", false).await;

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");
    manager
        .grant_all_plugin_permissions("demo.permissions")
        .await
        .expect("grant first version");

    write_permission_manifest(&package_dir, "2.0.0", false).await;
    manager
        .update_plugin("demo.permissions", true)
        .await
        .expect("update package");

    let status = manager
        .permission_status("demo.permissions")
        .await
        .expect("permission status after version-only update");
    assert!(!status.grant_is_current);
    assert!(status.granted.is_empty());
    assert!(manager
        .ensure_plugin_permission("demo.permissions", crate::plugins::PluginPermission::FsRead)
        .await
        .is_err());
}

async fn write_permission_manifest(package_dir: &std::path::Path, version: &str, fs_write: bool) {
    fs::write(
        package_dir.join("plugin.toml"),
        format!(
            r#"
manifest_version = 1
id = "demo.permissions"
name = "Permission Demo"
version = "{version}"
publisher = "demo.publisher"
runtime = "js"
entry_component = "entry.ts"

[requested_permissions]
fs_read = true
fs_write = {fs_write}
network = true
process = true
"#
        ),
    )
    .await
    .expect("write permission manifest");
}

#[tokio::test]
async fn uninstall_revokes_grants_and_reclaims_final_snapshot() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("uninstall-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("entry.ts"), "export {}")
        .await
        .expect("write entry");
    write_permission_manifest(&package_dir, "1.0.0", false).await;

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");
    manager
        .grant_all_plugin_permissions("demo.permissions")
        .await
        .expect("grant permissions");
    let lock = load_lockfile(&manager).await.expect("load lock");
    let managed_root = lock.plugins[0].managed_root.clone().expect("managed root");

    manager
        .uninstall_plugin("demo.permissions")
        .await
        .expect("uninstall");
    assert!(!managed_root.exists());
    assert!(manager.permission_status("demo.permissions").await.is_err());
    let permissions = storage::load_permissions(&manager)
        .await
        .expect("load permissions");
    assert!(!permissions.plugins.contains_key("demo.permissions"));
}

#[tokio::test]
async fn replacement_from_different_source_cannot_inherit_permission_grant() {
    let temp = tempdir().expect("tempdir");
    let first = temp.path().join("first-source");
    let second = temp.path().join("second-source");
    for package_dir in [&first, &second] {
        fs::create_dir_all(package_dir)
            .await
            .expect("create package");
        fs::write(package_dir.join("entry.ts"), "export {}")
            .await
            .expect("write entry");
        write_permission_manifest(package_dir, "1.0.0", false).await;
    }

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(first.to_str().expect("first path utf8"))
        .await
        .expect("install first source");
    manager
        .grant_all_plugin_permissions("demo.permissions")
        .await
        .expect("grant first source");
    assert!(
        manager
            .permission_status("demo.permissions")
            .await
            .expect("first status")
            .grant_is_current
    );

    manager
        .install_from_ref(second.to_str().expect("second path utf8"))
        .await
        .expect("replace from second source");
    assert!(
        !manager
            .permission_status("demo.permissions")
            .await
            .expect("replacement status")
            .grant_is_current
    );
}

#[tokio::test]
async fn uninstall_keeps_shared_snapshot_until_last_plugin_is_removed() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("multi-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("a.ts"), "export {}")
        .await
        .expect("write a");
    fs::write(package_dir.join("b.ts"), "export {}")
        .await
        .expect("write b");
    fs::write(
        package_dir.join("package.json"),
        r#"{"krusty":{"plugins":["a.toml","b.toml"]}}"#,
    )
    .await
    .expect("write package json");
    for (id, manifest, entry) in [("demo.a", "a.toml", "a.ts"), ("demo.b", "b.toml", "b.ts")] {
        fs::write(
            package_dir.join(manifest),
            format!(
                "manifest_version=1\nid=\"{id}\"\nname=\"{id}\"\nversion=\"1.0.0\"\npublisher=\"demo\"\nruntime=\"js\"\nentry_component=\"{entry}\"\n[requested_permissions]\nprocess=true\n"
            ),
        )
        .await
        .expect("write manifest");
    }

    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install package");
    let root = load_lockfile(&manager).await.unwrap().plugins[0]
        .managed_root
        .clone()
        .unwrap();

    manager.uninstall_plugin("demo.a").await.expect("remove a");
    assert!(root.exists());
    manager.uninstall_plugin("demo.b").await.expect("remove b");
    assert!(!root.exists());
}

#[tokio::test]
async fn reconcile_removes_orphan_transactions_and_snapshots() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let staging_orphan = manager.staging_root().join("orphan-stage");
    let managed_orphan = manager.managed_root().join("orphan-managed");
    fs::create_dir_all(&staging_orphan)
        .await
        .expect("create staging orphan");
    fs::create_dir_all(&managed_orphan)
        .await
        .expect("create managed orphan");

    let report = manager.reconcile_plugins(false).await.expect("reconcile");
    assert_eq!(report.removed_orphan_roots.len(), 2);
    assert!(!staging_orphan.exists());
    assert!(!managed_orphan.exists());
}

#[tokio::test]
async fn lock_manifest_identity_mismatch_is_rejected() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("identity-package");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("entry.ts"), "export {}")
        .await
        .expect("write entry");
    fs::write(
        package_dir.join("plugin.toml"),
        "manifest_version=1\nid=\"demo.identity\"\nname=\"Identity\"\nversion=\"1.0.0\"\npublisher=\"demo\"\nruntime=\"js\"\nentry_component=\"entry.ts\"\n[requested_permissions]\nprocess=true\n",
    )
    .await
    .expect("write manifest");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect("install");
    let lock = load_lockfile(&manager).await.expect("load lock");
    let entry = &lock.plugins[0];
    let manifest_path = entry
        .package_path
        .as_ref()
        .unwrap()
        .join(entry.manifest_path.as_ref().unwrap());
    let contents = fs::read_to_string(&manifest_path).await.unwrap();
    fs::write(
        &manifest_path,
        contents.replace("demo.identity", "demo.impostor"),
    )
    .await
    .expect("tamper manifest");

    let error = read_installed_from_lock_entry(&manager, entry)
        .await
        .expect_err("identity mismatch should fail");
    assert!(error.to_string().contains("lockfile identity mismatch"));
}

#[tokio::test]
async fn rejects_plain_http_catalog_sources() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let error = manager
        .add_source(Some("insecure"), "http://example.com/catalog.json")
        .await
        .expect_err("plain HTTP must fail");
    assert!(error.to_string().contains("must use HTTPS"));
}

#[tokio::test]
async fn rejects_executable_bundle_without_process_declaration() {
    let temp = tempdir().expect("tempdir");
    let package_dir = temp.path().join("unsafe-bundle");
    fs::create_dir_all(&package_dir)
        .await
        .expect("create package");
    fs::write(package_dir.join("extension.ts"), "export {}")
        .await
        .expect("write extension");
    fs::write(
        package_dir.join("plugin.toml"),
        r#"
manifest_version = 1
id = "demo.unsafe-bundle"
name = "Unsafe Bundle"
version = "1.0.0"
publisher = "demo"
agent_extensions = ["extension.ts"]
"#,
    )
    .await
    .expect("write manifest");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let error = manager
        .install_from_ref(package_dir.to_str().expect("package path utf8"))
        .await
        .expect_err("process declaration should be required");
    assert!(error
        .to_string()
        .contains("requested_permissions.process = true"));
}

#[tokio::test]
async fn stable_os_mutation_lock_serializes_independent_managers() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("plugins");
    let first = PluginManager::new(reqwest::Client::new(), root.clone());
    let second = PluginManager::new(reqwest::Client::new(), root);
    first.ensure_layout().await.expect("ensure layout");

    let lock_path = first.root().join(".mutation.lock");
    fs::write(&lock_path, "legacy-stale-lease-metadata")
        .await
        .expect("write legacy lease file");
    let first_guard = first.acquire_mutation().await.expect("acquire first lock");

    let mut waiter = tokio::spawn(async move { second.acquire_mutation().await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut waiter)
            .await
            .is_err(),
        "an independent manager must not enter while the OS lock is held"
    );

    drop(first_guard);
    let second_guard = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("second manager should acquire after descriptor close")
        .expect("waiter task should complete")
        .expect("acquire second lock");
    drop(second_guard);

    assert!(
        lock_path.exists(),
        "the stable lock inode must not be unlinked"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).await.unwrap(),
        "legacy-stale-lease-metadata",
        "lock ownership must not depend on stale path contents"
    );
}

#[tokio::test]
async fn concurrent_managers_do_not_lose_plugin_state_updates() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("plugins");
    let manager = PluginManager::new(reqwest::Client::new(), root.clone());
    manager.ensure_layout().await.expect("ensure layout");

    let mut updates = Vec::new();
    for index in 0..8 {
        let independent_manager = PluginManager::new(reqwest::Client::new(), root.clone());
        updates.push(tokio::spawn(async move {
            independent_manager
                .add_source(
                    Some(&format!("source-{index}")),
                    &format!("https://example.com/catalog-{index}.json"),
                )
                .await
        }));
    }
    for update in updates {
        update
            .await
            .expect("state update task should complete")
            .expect("state update should succeed");
    }

    let sources = manager.list_sources().await.expect("load sources");
    assert_eq!(sources.len(), 8);
    for index in 0..8 {
        assert!(sources
            .iter()
            .any(|source| source.name == format!("source-{index}")));
    }
}

#[tokio::test]
async fn absent_plugin_state_loads_defaults_without_writing_files() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    assert_eq!(
        storage::load_lockfile(&manager).await.unwrap(),
        crate::plugins::PluginLockfile::default()
    );
    assert_eq!(
        storage::load_trust_policy(&manager).await.unwrap(),
        crate::plugins::PluginTrustPolicy::default()
    );
    assert_eq!(
        storage::load_permissions(&manager).await.unwrap(),
        crate::plugins::PluginPermissionsFile::default()
    );
    assert_eq!(
        storage::load_sources(&manager).await.unwrap(),
        crate::plugins::PluginSourcesFile::default()
    );

    for path in [
        manager.lockfile_path(),
        manager.trust_file_path(),
        manager.permissions_file_path(),
        manager.sources_file_path(),
    ] {
        assert!(
            !path.exists(),
            "an unguarded default read must not create {}",
            path.display()
        );
    }
}

#[tokio::test]
async fn legacy_plugin_state_without_version_is_treated_as_v1() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    fs::write(manager.lockfile_path(), "plugins = []\n")
        .await
        .unwrap();
    fs::write(
        manager.trust_file_path(),
        "allowed_publishers = []\n[keys]\n[publisher_keys]\n",
    )
    .await
    .unwrap();
    fs::write(manager.permissions_file_path(), "[plugins]\n")
        .await
        .unwrap();
    fs::write(manager.sources_file_path(), "sources = []\n")
        .await
        .unwrap();

    assert_eq!(storage::load_lockfile(&manager).await.unwrap().version, 1);
    storage::load_trust_policy(&manager).await.unwrap();
    manager
        .add_allowed_publisher("legacy.publisher")
        .await
        .unwrap();
    assert!(
        fs::read_to_string(manager.trust_file_path())
            .await
            .unwrap()
            .contains("version = 1"),
        "the next guarded trust mutation should upgrade legacy state to explicit v1"
    );
    assert_eq!(
        storage::load_permissions(&manager).await.unwrap().version,
        1
    );
    assert_eq!(storage::load_sources(&manager).await.unwrap().version, 1);
}

#[tokio::test]
async fn future_plugin_state_versions_fail_closed_before_mutation() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let cases = [
        (manager.lockfile_path(), "version = 99\nplugins = []\n"),
        (
            manager.trust_file_path(),
            "version = 99\nallowed_publishers = []\n[keys]\n[publisher_keys]\n",
        ),
        (manager.permissions_file_path(), "version = 99\n[plugins]\n"),
        (manager.sources_file_path(), "version = 99\nsources = []\n"),
    ];
    for (path, contents) in &cases {
        fs::write(path, contents).await.unwrap();
    }

    for (label, result) in [
        (
            "plugin lockfile",
            storage::load_lockfile(&manager).await.map(|_| ()),
        ),
        (
            "plugin trust policy",
            storage::load_trust_policy(&manager).await.map(|_| ()),
        ),
        (
            "plugin permissions",
            storage::load_permissions(&manager).await.map(|_| ()),
        ),
        (
            "plugin sources",
            storage::load_sources(&manager).await.map(|_| ()),
        ),
    ] {
        let error = result.expect_err("future state schema must be rejected");
        assert!(
            error.to_string().contains(label),
            "unexpected error: {error:#}"
        );
        assert!(
            error.to_string().contains("schema version 99"),
            "unexpected error: {error:#}"
        );
    }

    let error = manager
        .add_allowed_publisher("must-not-be-written")
        .await
        .expect_err("mutation must fail on future trust schema");
    assert!(error.to_string().contains("schema version 99"));
    assert_eq!(
        fs::read_to_string(manager.trust_file_path()).await.unwrap(),
        cases[1].1,
        "failed validation must leave the future state untouched"
    );

    let future_permissions = crate::plugins::PluginPermissionsFile {
        version: 99,
        ..Default::default()
    };
    let error = storage::save_permissions(&manager, &future_permissions)
        .await
        .expect_err("save must reject unsupported in-memory schema");
    assert!(error.to_string().contains("schema version 99"));
}

#[cfg(unix)]
#[tokio::test]
async fn active_cleanup_rejects_swapped_root_without_touching_victim_tree() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let victim = temp.path().join("victim");
    fs::create_dir_all(victim.join("demo.active"))
        .await
        .expect("create victim tree");
    fs::write(victim.join("demo.active/keep.txt"), b"keep")
        .await
        .expect("write victim marker");
    fs::remove_dir(manager.active_root())
        .await
        .expect("remove original active root");
    symlink(&victim, manager.active_root()).expect("swap active root for hostile symlink");

    let error = lifecycle::remove_active_entry(&manager, "demo.active")
        .await
        .expect_err("a swapped active root must fail closed");
    assert!(error
        .to_string()
        .contains("active plugin root must be a real directory"));
    assert_eq!(
        fs::read(victim.join("demo.active/keep.txt"))
            .await
            .expect("victim marker remains"),
        b"keep"
    );
}

#[tokio::test]
async fn active_cleanup_never_recursively_removes_nonempty_state() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let active_entry = manager.active_root().join("demo.active");
    fs::create_dir(&active_entry)
        .await
        .expect("create active entry");
    fs::write(active_entry.join("keep.txt"), b"keep")
        .await
        .expect("write active marker");

    lifecycle::remove_active_entry(&manager, "demo.active")
        .await
        .expect_err("nonempty active state must be left for explicit reconciliation");
    assert_eq!(
        fs::read(active_entry.join("keep.txt"))
            .await
            .expect("active marker remains"),
        b"keep"
    );
}

#[tokio::test]
async fn package_manifest_fanout_and_duplicate_paths_fail_early() {
    let temp = tempdir().expect("tempdir");
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let fanout_package = temp.path().join("fanout-package");
    fs::create_dir(&fanout_package)
        .await
        .expect("create fanout package");
    let paths: Vec<_> = (0..257).map(|index| format!("{index}.toml")).collect();
    fs::write(
        fanout_package.join("package.json"),
        serde_json::to_vec(&serde_json::json!({ "krusty": { "plugins": paths } })).unwrap(),
    )
    .await
    .expect("write fanout package.json");
    let error = manager
        .install_from_ref(fanout_package.to_str().unwrap())
        .await
        .expect_err("manifest fanout above 256 must fail");
    assert!(error.to_string().contains("the limit is 256"));

    let duplicate_package = temp.path().join("duplicate-package");
    fs::create_dir(&duplicate_package)
        .await
        .expect("create duplicate package");
    fs::write(
        duplicate_package.join("package.json"),
        br#"{"krusty":{"plugins":["missing.toml","./missing.toml"]}}"#,
    )
    .await
    .expect("write duplicate package.json");
    let error = manager
        .install_from_ref(duplicate_package.to_str().unwrap())
        .await
        .expect_err("normalized duplicate manifest paths must fail before file access");
    assert!(error
        .to_string()
        .contains("contains duplicate manifest path 'missing.toml'"));
}

#[tokio::test]
async fn package_manifest_aggregate_bytes_are_bounded() {
    let temp = tempdir().expect("tempdir");
    let package = temp.path().join("aggregate-package");
    fs::create_dir_all(package.join("shared-skill"))
        .await
        .expect("create package components");
    fs::write(package.join("shared-skill/SKILL.md"), "# Shared")
        .await
        .expect("write component");
    let paths: Vec<_> = (0..9).map(|index| format!("{index}.toml")).collect();
    fs::write(
        package.join("package.json"),
        serde_json::to_vec(&serde_json::json!({ "krusty": { "plugins": paths } })).unwrap(),
    )
    .await
    .expect("write package.json");
    let padding = "x".repeat(950_000);
    for index in 0..9 {
        fs::write(
            package.join(format!("{index}.toml")),
            format!(
                "manifest_version=1\nid=\"demo.aggregate-{index}\"\nname=\"Aggregate {index}\"\nversion=\"1.0.0\"\npublisher=\"demo\"\nskills=[\"shared-skill\"]\n#{padding}\n"
            ),
        )
        .await
        .expect("write padded manifest");
    }
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");

    let error = manager
        .install_from_ref(package.to_str().unwrap())
        .await
        .expect_err("aggregate manifest bytes above the budget must fail");
    assert!(error
        .to_string()
        .contains("manifests exceed aggregate size limit"));
}

#[tokio::test]
async fn exact_descriptor_permission_check_rejects_same_id_different_version() {
    let temp = tempdir().expect("tempdir");
    let package = temp.path().join("permission-package");
    fs::create_dir(&package).await.expect("create package");
    fs::write(package.join("entry.ts"), "export {}")
        .await
        .expect("write entry");
    write_permission_manifest(&package, "1.0.0", false).await;
    let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
    manager.ensure_layout().await.expect("ensure layout");
    let installed = manager
        .install_from_ref(package.to_str().unwrap())
        .await
        .expect("install plugin")
        .remove(0);
    manager
        .grant_all_plugin_permissions(&installed.id)
        .await
        .expect("grant descriptor permissions");

    manager
        .ensure_installed_plugin_permission(&installed, crate::plugins::PluginPermission::FsRead)
        .await
        .expect("the reviewed descriptor should be authorized");
    let mut substituted = installed.clone();
    substituted.version = "2.0.0".into();
    let error = manager
        .ensure_installed_plugin_permission(&substituted, crate::plugins::PluginPermission::FsRead)
        .await
        .expect_err("same ID with a different version must not inherit activation authority");
    assert!(error.to_string().contains("exact plugin descriptor"));
}
