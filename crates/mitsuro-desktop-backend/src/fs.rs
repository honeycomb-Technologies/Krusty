//! `fs/*` and `fuzzyFileSearch*` app-server methods.
//!
//! Typed Codex app-server filesystem and fuzzy-search shapes.
//!
//! - Client methods: `fs/readDirectory`, `fs/readFile`, `fs/getMetadata`,
//!   `fuzzyFileSearch`, `fuzzyFileSearch/sessionStart|sessionUpdate|sessionStop`
//! - Fixture virtual tree root: [`FIXTURE_PROJECT_ROOT`] (`/fixture-project/`)

use serde::{Deserialize, Serialize};

use crate::process::{decode_base64, encode_base64};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Absolute root of the offline fixture project tree (virtual, not on disk).
pub const FIXTURE_PROJECT_ROOT: &str = "/fixture-project";

// ---------------------------------------------------------------------------
// fs/readDirectory
// ---------------------------------------------------------------------------

/// Params for `fs/readDirectory`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryParams {
    /// Absolute directory path to read.
    pub path: String,
}

impl FsReadDirectoryParams {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// A directory entry returned by `fs/readDirectory`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryEntry {
    /// Direct child entry name only (not a path).
    pub file_name: String,
    pub is_directory: bool,
    pub is_file: bool,
}

impl FsReadDirectoryEntry {
    pub fn file(name: impl Into<String>) -> Self {
        Self {
            file_name: name.into(),
            is_directory: false,
            is_file: true,
        }
    }

    pub fn directory(name: impl Into<String>) -> Self {
        Self {
            file_name: name.into(),
            is_directory: true,
            is_file: false,
        }
    }
}

/// Response for `fs/readDirectory`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsReadDirectoryResponse {
    pub entries: Vec<FsReadDirectoryEntry>,
}

// ---------------------------------------------------------------------------
// fs/readFile
// ---------------------------------------------------------------------------

/// Params for `fs/readFile`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsReadFileParams {
    pub path: String,
}

impl FsReadFileParams {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Base64-encoded file contents from `fs/readFile`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsReadFileResponse {
    pub data_base64: String,
}

impl FsReadFileResponse {
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            data_base64: encode_base64(data),
        }
    }

    pub fn from_text(text: &str) -> Self {
        Self::from_bytes(text.as_bytes())
    }

    /// Decode payload to lossy UTF-8 (UI preview helper).
    pub fn text_lossy(&self) -> String {
        match decode_base64(&self.data_base64) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// fs/getMetadata
// ---------------------------------------------------------------------------

/// Params for `fs/getMetadata`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsGetMetadataParams {
    pub path: String,
}

impl FsGetMetadataParams {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Metadata returned by `fs/getMetadata`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsGetMetadataResponse {
    pub created_at_ms: i64,
    pub is_directory: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub modified_at_ms: i64,
}

// ---------------------------------------------------------------------------
// fuzzyFileSearch
// ---------------------------------------------------------------------------

/// Match kind for fuzzy file search hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuzzyFileSearchMatchType {
    File,
    Directory,
}

impl FuzzyFileSearchMatchType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

/// One fuzzy search hit (wire uses snake_case field names).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyFileSearchResult {
    pub root: String,
    pub path: String,
    pub match_type: FuzzyFileSearchMatchType,
    pub file_name: String,
    pub score: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<Vec<u32>>,
}

/// Params for one-shot `fuzzyFileSearch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuzzyFileSearchParams {
    pub query: String,
    pub roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation_token: Option<String>,
}

impl FuzzyFileSearchParams {
    pub fn new(query: impl Into<String>, roots: Vec<String>) -> Self {
        Self {
            query: query.into(),
            roots,
            cancellation_token: None,
        }
    }
}

/// Response for `fuzzyFileSearch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyFileSearchResponse {
    pub files: Vec<FuzzyFileSearchResult>,
}

// ---------------------------------------------------------------------------
// fuzzyFileSearch session variants
// ---------------------------------------------------------------------------

/// Params for `fuzzyFileSearch/sessionStart`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuzzyFileSearchSessionStartParams {
    pub roots: Vec<String>,
    pub session_id: String,
}

impl FuzzyFileSearchSessionStartParams {
    pub fn new(session_id: impl Into<String>, roots: Vec<String>) -> Self {
        Self {
            roots,
            session_id: session_id.into(),
        }
    }
}

/// Empty success for `fuzzyFileSearch/sessionStart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyFileSearchSessionStartResponse {}

/// Params for `fuzzyFileSearch/sessionUpdate`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuzzyFileSearchSessionUpdateParams {
    pub query: String,
    pub session_id: String,
}

impl FuzzyFileSearchSessionUpdateParams {
    pub fn new(session_id: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            session_id: session_id.into(),
        }
    }
}

/// Empty success for `fuzzyFileSearch/sessionUpdate` (results arrive via notification).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyFileSearchSessionUpdateResponse {}

/// Params for `fuzzyFileSearch/sessionStop`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuzzyFileSearchSessionStopParams {
    pub session_id: String,
}

impl FuzzyFileSearchSessionStopParams {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

/// Empty success for `fuzzyFileSearch/sessionStop`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyFileSearchSessionStopResponse {}

// ---------------------------------------------------------------------------
// Fuzzy matching helpers (name-focused subsequence scorer)
// ---------------------------------------------------------------------------

/// Score a candidate name against a query (higher is better).
///
/// Returns `None` when the query characters are not a subsequence of `name`
/// (case-insensitive). Empty query matches everything with score 0.
pub fn fuzzy_score_name(query: &str, name: &str) -> Option<(u32, Vec<u32>)> {
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let n: Vec<char> = name.chars().collect();
    let mut indices = Vec::with_capacity(q.len());
    let mut qi = 0usize;
    let mut consecutive_bonus: u32 = 0;
    let mut score: u32 = 0;
    for (ni, &ch) in n.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if ch.eq_ignore_ascii_case(&q[qi]) {
            indices.push(ni as u32);
            score = score.saturating_add(10);
            if qi > 0 && indices.len() >= 2 {
                let prev = indices[indices.len() - 2];
                if ni as u32 == prev + 1 {
                    consecutive_bonus = consecutive_bonus.saturating_add(5);
                }
            }
            // Prefer matches at start / after separator.
            if ni == 0
                || n.get(ni.saturating_sub(1))
                    .is_some_and(|c| *c == '_' || *c == '-' || *c == '.' || *c == '/')
            {
                score = score.saturating_add(4);
            }
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    // Prefer shorter names when scores tie-break via remaining length penalty.
    let length_penalty = (n.len() as u32).saturating_sub(q.len() as u32).min(20);
    let total = score
        .saturating_add(consecutive_bonus)
        .saturating_sub(length_penalty);
    Some((total.max(1), indices))
}

/// Normalize a virtual absolute path (collapse `//`, strip trailing slash except root).
pub fn normalize_abs_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut prev_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push('/');
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    if out.is_empty() {
        "/".into()
    } else {
        out
    }
}

/// Join parent absolute path + child name.
pub fn join_abs(parent: &str, name: &str) -> String {
    let p = normalize_abs_path(parent);
    if p == "/" {
        format!("/{name}")
    } else {
        format!("{p}/{name}")
    }
}

// ---------------------------------------------------------------------------
// Fixture virtual filesystem
// ---------------------------------------------------------------------------

/// In-memory node in the fixture project tree.
#[derive(Debug, Clone)]
pub enum FixtureFsNode {
    File {
        content: String,
        created_at_ms: i64,
        modified_at_ms: i64,
    },
    Dir {
        children: Vec<(String, FixtureFsNode)>,
        created_at_ms: i64,
        modified_at_ms: i64,
    },
}

impl FixtureFsNode {
    fn file(content: impl Into<String>) -> Self {
        let t = 1_722_700_000_000i64;
        Self::File {
            content: content.into(),
            created_at_ms: t,
            modified_at_ms: t + 3_600_000,
        }
    }

    fn dir(children: Vec<(String, FixtureFsNode)>) -> Self {
        let t = 1_722_700_000_000i64;
        Self::Dir {
            children,
            created_at_ms: t,
            modified_at_ms: t + 1_800_000,
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self, Self::Dir { .. })
    }

    #[allow(dead_code)]
    fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }
}

/// Build the default `/fixture-project` virtual tree used by [`crate::fixture::FixtureBackend`].
pub fn fixture_project_tree() -> FixtureFsNode {
    FixtureFsNode::dir(vec![(
        "fixture-project".into(),
        FixtureFsNode::dir(vec![
            (
                "README.md".into(),
                FixtureFsNode::file(
                    "# Fixture Project\n\nOffline Mitsuro sample tree for fs/* and fuzzyFileSearch.\n",
                ),
            ),
            (
                "Cargo.toml".into(),
                FixtureFsNode::file(
                    "[package]\nname = \"fixture-project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                ),
            ),
            (
                "src".into(),
                FixtureFsNode::dir(vec![
                    (
                        "main.rs".into(),
                        FixtureFsNode::file(
                            "fn main() {\n    println!(\"hello from fixture-project\");\n}\n",
                        ),
                    ),
                    (
                        "lib.rs".into(),
                        FixtureFsNode::file(
                            "//! Fixture library.\n\npub fn greet(name: &str) -> String {\n    format!(\"hello, {name}\")\n}\n",
                        ),
                    ),
                    (
                        "utils.rs".into(),
                        FixtureFsNode::file("pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"),
                    ),
                ]),
            ),
            (
                "docs".into(),
                FixtureFsNode::dir(vec![(
                    "guide.md".into(),
                    FixtureFsNode::file(
                        "# Guide\n\nUse the Files panel to browse and fuzzy-search this tree.\n",
                    ),
                )]),
            ),
            (
                ".mitsuro".into(),
                FixtureFsNode::dir(vec![(
                    "notes.txt".into(),
                    FixtureFsNode::file("Mitsuro fixture notes — not a real filesystem path.\n"),
                )]),
            ),
        ]),
    )])
}

/// Walk `root` (the synthetic `/` node whose child is `fixture-project`) to `path`.
pub fn fixture_lookup<'a>(root: &'a FixtureFsNode, path: &str) -> Option<&'a FixtureFsNode> {
    let path = normalize_abs_path(path);
    if path == "/" {
        return Some(root);
    }
    let mut cur = root;
    for seg in path.trim_start_matches('/').split('/') {
        if seg.is_empty() {
            continue;
        }
        match cur {
            FixtureFsNode::Dir { children, .. } => {
                cur = children.iter().find(|(n, _)| n == seg).map(|(_, n)| n)?;
            }
            FixtureFsNode::File { .. } => return None,
        }
    }
    Some(cur)
}

/// List directory entries for a path under the fixture tree.
pub fn fixture_read_directory(
    root: &FixtureFsNode,
    path: &str,
) -> Result<FsReadDirectoryResponse, String> {
    let node = fixture_lookup(root, path).ok_or_else(|| format!("path not found: {path}"))?;
    match node {
        FixtureFsNode::Dir { children, .. } => {
            let mut entries: Vec<FsReadDirectoryEntry> = children
                .iter()
                .map(|(name, child)| {
                    if child.is_dir() {
                        FsReadDirectoryEntry::directory(name.clone())
                    } else {
                        FsReadDirectoryEntry::file(name.clone())
                    }
                })
                .collect();
            entries.sort_by(|a, b| {
                // Directories first, then name.
                match (a.is_directory, b.is_directory) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.file_name.cmp(&b.file_name),
                }
            });
            Ok(FsReadDirectoryResponse { entries })
        }
        FixtureFsNode::File { .. } => Err(format!("not a directory: {path}")),
    }
}

/// Read file bytes (as base64 response) for a path under the fixture tree.
pub fn fixture_read_file(root: &FixtureFsNode, path: &str) -> Result<FsReadFileResponse, String> {
    let node = fixture_lookup(root, path).ok_or_else(|| format!("path not found: {path}"))?;
    match node {
        FixtureFsNode::File { content, .. } => Ok(FsReadFileResponse::from_text(content)),
        FixtureFsNode::Dir { .. } => Err(format!("is a directory: {path}")),
    }
}

/// Metadata for a path under the fixture tree.
pub fn fixture_get_metadata(
    root: &FixtureFsNode,
    path: &str,
) -> Result<FsGetMetadataResponse, String> {
    let node = fixture_lookup(root, path).ok_or_else(|| format!("path not found: {path}"))?;
    match node {
        FixtureFsNode::File {
            created_at_ms,
            modified_at_ms,
            ..
        } => Ok(FsGetMetadataResponse {
            created_at_ms: *created_at_ms,
            is_directory: false,
            is_file: true,
            is_symlink: false,
            modified_at_ms: *modified_at_ms,
        }),
        FixtureFsNode::Dir {
            created_at_ms,
            modified_at_ms,
            ..
        } => Ok(FsGetMetadataResponse {
            created_at_ms: *created_at_ms,
            is_directory: true,
            is_file: false,
            is_symlink: false,
            modified_at_ms: *modified_at_ms,
        }),
    }
}

/// Flatten all file/dir paths under `root_path` for fuzzy search.
pub fn fixture_collect_entries(
    root: &FixtureFsNode,
    root_path: &str,
) -> Vec<(String, String, FuzzyFileSearchMatchType)> {
    let mut out = Vec::new();
    let Some(node) = fixture_lookup(root, root_path) else {
        return out;
    };
    collect_recursive(node, root_path, &mut out);
    out
}

fn collect_recursive(
    node: &FixtureFsNode,
    path: &str,
    out: &mut Vec<(String, String, FuzzyFileSearchMatchType)>,
) {
    match node {
        FixtureFsNode::File { .. } => {
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            out.push((path.to_string(), name, FuzzyFileSearchMatchType::File));
        }
        FixtureFsNode::Dir { children, .. } => {
            // Include the directory itself (except pure root) when it has a name.
            if path != "/" {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                out.push((path.to_string(), name, FuzzyFileSearchMatchType::Directory));
            }
            for (child_name, child) in children {
                let child_path = join_abs(path, child_name);
                collect_recursive(child, &child_path, out);
            }
        }
    }
}

/// Run fuzzy search over fixture entries under the given roots.
pub fn fixture_fuzzy_search(
    root: &FixtureFsNode,
    query: &str,
    roots: &[String],
) -> FuzzyFileSearchResponse {
    let mut files = Vec::new();
    for search_root in roots {
        let search_root = normalize_abs_path(search_root);
        let entries = fixture_collect_entries(root, &search_root);
        for (path, file_name, match_type) in entries {
            // Score primarily on file name; fall back to full relative path.
            let scored = fuzzy_score_name(query, &file_name)
                .or_else(|| fuzzy_score_name(query, path.trim_start_matches(&search_root)));
            if let Some((score, indices)) = scored {
                // Empty query: list everything with zero score (browse assist).
                files.push(FuzzyFileSearchResult {
                    root: search_root.clone(),
                    path,
                    match_type,
                    file_name,
                    score,
                    indices: if indices.is_empty() {
                        None
                    } else {
                        Some(indices)
                    },
                });
            }
        }
    }
    // Sort by score desc, then path.
    files.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    // Cap results for UI sanity.
    if files.len() > 100 {
        files.truncate(100);
    }
    FuzzyFileSearchResponse { files }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_tree_read_directory_root() {
        let tree = fixture_project_tree();
        let resp = fixture_read_directory(&tree, FIXTURE_PROJECT_ROOT).unwrap();
        let names: Vec<_> = resp.entries.iter().map(|e| e.file_name.as_str()).collect();
        assert!(names.contains(&"README.md"));
        assert!(names.contains(&"src"));
        assert!(names.contains(&"docs"));
        assert!(names.contains(&"Cargo.toml"));
        let src = resp.entries.iter().find(|e| e.file_name == "src").unwrap();
        assert!(src.is_directory);
        assert!(!src.is_file);
    }

    #[test]
    fn fixture_tree_read_file_and_metadata() {
        let tree = fixture_project_tree();
        let path = format!("{FIXTURE_PROJECT_ROOT}/src/main.rs");
        let file = fixture_read_file(&tree, &path).unwrap();
        let text = file.text_lossy();
        assert!(text.contains("hello from fixture-project"));

        let meta = fixture_get_metadata(&tree, &path).unwrap();
        assert!(meta.is_file);
        assert!(!meta.is_directory);
        assert!(!meta.is_symlink);
        assert!(meta.modified_at_ms > 0);

        let dir_meta = fixture_get_metadata(&tree, FIXTURE_PROJECT_ROOT).unwrap();
        assert!(dir_meta.is_directory);
    }

    #[test]
    fn fuzzy_filters_names() {
        let tree = fixture_project_tree();
        let resp = fixture_fuzzy_search(&tree, "main", &[FIXTURE_PROJECT_ROOT.to_string()]);
        assert!(
            resp.files.iter().any(|f| f.file_name == "main.rs"),
            "expected main.rs in {:?}",
            resp.files
        );
        assert!(
            !resp.files.iter().any(|f| f.file_name == "guide.md"),
            "guide.md should not match 'main'"
        );

        let resp2 = fixture_fuzzy_search(&tree, "gmd", &[FIXTURE_PROJECT_ROOT.to_string()]);
        // guide.md — g, m, d subsequence may match guide.md (g-u-i-d-e . m-d)
        assert!(
            resp2
                .files
                .iter()
                .any(|f| f.file_name.contains("guide") || f.file_name.ends_with(".md")),
            "expected a md-ish hit for gmd: {:?}",
            resp2.files
        );

        let resp3 = fixture_fuzzy_search(&tree, "librs", &[FIXTURE_PROJECT_ROOT.to_string()]);
        assert!(
            resp3.files.iter().any(|f| f.file_name == "lib.rs"),
            "librs should fuzzy-match lib.rs: {:?}",
            resp3.files
        );
    }

    #[test]
    fn serialize_camel_case_fs_params() {
        let p = FsReadDirectoryParams::new("/fixture-project");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["path"], "/fixture-project");

        let r = FsReadFileResponse::from_text("hi");
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("dataBase64").is_some());

        let f = FuzzyFileSearchParams::new("main", vec!["/fixture-project".into()]);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["query"], "main");
        assert!(v["roots"].as_array().unwrap().len() == 1);

        let hit = FuzzyFileSearchResult {
            root: "/fixture-project".into(),
            path: "/fixture-project/src/main.rs".into(),
            match_type: FuzzyFileSearchMatchType::File,
            file_name: "main.rs".into(),
            score: 42,
            indices: Some(vec![0, 1, 2, 3]),
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v["file_name"], "main.rs");
        assert_eq!(v["match_type"], "file");
        assert_eq!(v["score"], 42);
    }

    #[test]
    fn normalize_and_join() {
        assert_eq!(normalize_abs_path("/fixture-project/"), "/fixture-project");
        assert_eq!(normalize_abs_path("//a//b"), "/a/b");
        assert_eq!(join_abs("/fixture-project", "src"), "/fixture-project/src");
        assert_eq!(join_abs("/", "fixture-project"), "/fixture-project");
    }
}
