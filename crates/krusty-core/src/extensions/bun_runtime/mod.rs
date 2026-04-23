//! Bun runtime management for extensions
//!
//! Replaces Node.js with Bun for faster JavaScript runtime.

mod commands;
mod install;

use serde::Deserialize;
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

const BUN_VERSION: &str = "1.1.42";

#[cfg(not(windows))]
const BUN_BINARY: &str = "bun";
#[cfg(windows)]
const BUN_BINARY: &str = "bun.exe";

/// Bun runtime manager
#[derive(Clone)]
pub struct BunRuntime {
    inner: Arc<RwLock<BunRuntimeInner>>,
    http_client: reqwest::Client,
    data_dir: PathBuf,
}

struct BunRuntimeInner {
    instance: Option<BunInstance>,
}

#[derive(Clone, Debug)]
pub(crate) enum BunInstance {
    System { bun: PathBuf },
    Managed { installation_path: PathBuf },
}

fn path_with_bun_prepended(bun_binary: &Path) -> Option<OsString> {
    let existing_path = env::var_os("PATH")?;
    let bun_dir = bun_binary.parent()?;

    env::join_paths(std::iter::once(bun_dir.to_path_buf()).chain(env::split_paths(&existing_path)))
        .ok()
}

#[derive(Deserialize)]
struct BunPackageInfo {
    version: Option<String>,
    #[serde(rename = "dist-tags")]
    dist_tags: Option<DistTags>,
}

#[derive(Deserialize)]
struct DistTags {
    latest: Option<String>,
}

#[derive(Deserialize)]
struct PackageJson {
    version: String,
}
