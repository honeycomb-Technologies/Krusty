use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest as _, Sha256};
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
    assert_eq!(
        installed[0].install_path,
        fs::canonicalize(&package_dir).await.unwrap()
    );
    assert!(installed[0]
        .entry_component_path
        .ends_with("dist/linux-x64/libdemo_plugin.so"));

    let plugins = manager
        .list_installed_plugins()
        .await
        .expect("list installed");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "demo.native");
}

#[tokio::test]
async fn local_package_install_runs_build_script_when_entry_is_missing() {
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
    assert_eq!(installed[0].id, "demo.buildable");
    assert!(installed[0].entry_component_path.exists());
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
      "tags": ["demo"]
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
    assert!(entries.iter().any(|entry| entry.id == "catalog.demo"));
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
