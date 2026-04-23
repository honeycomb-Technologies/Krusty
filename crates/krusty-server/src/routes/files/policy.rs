use crate::DEFAULT_MAX_REQUEST_BODY_BYTES;

#[derive(Debug, Clone, Copy)]
pub(super) struct FilesPolicy {
    max_text_file_bytes: usize,
    max_tree_entries: usize,
}

impl Default for FilesPolicy {
    fn default() -> Self {
        Self {
            // File writes intentionally share the server upload ceiling, and the
            // text read path uses the same bound to avoid asymmetric behavior.
            max_text_file_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            max_tree_entries: 10_000,
        }
    }
}

impl FilesPolicy {
    pub(super) fn exceeds_text_file_limit(self, bytes: u64) -> bool {
        bytes > self.max_text_file_bytes as u64
    }

    pub(super) fn max_tree_entries(self) -> usize {
        self.max_tree_entries
    }

    pub(super) fn workspace_tree_hides(self, name: &str) -> bool {
        name.starts_with('.') || matches!(name, "node_modules" | "target")
    }

    pub(super) fn browse_hides(self, name: &str) -> bool {
        name.starts_with('.')
    }

    pub(super) fn read_limit_error(self) -> String {
        format!(
            "File exceeds maximum text size of {}MB",
            self.max_text_file_bytes / (1024 * 1024)
        )
    }

    pub(super) fn write_limit_error(self) -> String {
        format!(
            "File content exceeds maximum text size of {}MB",
            self.max_text_file_bytes / (1024 * 1024)
        )
    }

    pub(super) fn non_utf8_error(self) -> &'static str {
        "File is not valid UTF-8 text; binary files are not supported by this endpoint"
    }
}
