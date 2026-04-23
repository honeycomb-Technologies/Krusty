use serde::{Deserialize, Serialize};

// ============================================================================
// File Types
// ============================================================================

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Deserialize)]
pub struct TreeQuery {
    pub root: Option<String>,
    #[serde(default = "default_depth", deserialize_with = "clamp_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    3
}

/// Maximum tree depth to prevent DoS via deep recursion
const MAX_TREE_DEPTH: usize = 10;

fn clamp_depth<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: usize = serde::Deserialize::deserialize(deserializer)?;
    Ok(value.min(MAX_TREE_DEPTH))
}

#[derive(Serialize)]
pub struct FileResponse {
    pub path: String,
    pub content: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct FileWriteRequest {
    pub content: String,
}

#[derive(Serialize)]
pub struct FileWriteResponse {
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeEntry>>,
}

#[derive(Serialize)]
pub struct TreeResponse {
    pub root: String,
    pub entries: Vec<TreeEntry>,
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    /// Directory to list (defaults to home directory)
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct BrowseEntry {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub directories: Vec<BrowseEntry>,
}
