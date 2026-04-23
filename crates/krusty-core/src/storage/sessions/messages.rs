use anyhow::Result;

use super::SessionManager;

impl SessionManager {
    /// Save a message to a session
    /// The content field stores JSON-serialized Vec<Content> for full fidelity
    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).save_message(
            session_id,
            role,
            content_json,
        )
    }

    /// Replace every persisted message for a session with a new ordered set.
    pub fn replace_session_messages(
        &self,
        session_id: &str,
        messages: &[(String, String)],
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db)
            .replace_session_messages(session_id, messages)
    }

    /// Update the most recent message of a given role in a session
    pub fn update_last_message(
        &self,
        session_id: &str,
        role: &str,
        content_json: &str,
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).update_last_message(
            session_id,
            role,
            content_json,
        )
    }

    /// Load all messages for a session
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        super::super::messages::MessageStore::new(&self.db).load_session_messages(session_id)
    }

    /// Generate a title from the first message content
    /// Truncates at word boundaries for cleaner display
    /// Uses char-based indexing for UTF-8 safety
    pub fn generate_title_from_content(content: &str) -> String {
        // Use first line only, cleaned up
        let first_line = content.lines().next().unwrap_or("").trim();

        // Count chars (not bytes) for UTF-8 safety
        let char_count = first_line.chars().count();

        // If short enough, use as-is
        if char_count <= 50 {
            return first_line.to_string();
        }

        // Get first 50 chars and find last word boundary
        let first_50: String = first_line.chars().take(50).collect();
        if let Some(last_space) = first_50.rfind(char::is_whitespace) {
            // last_space is a byte index in first_50, but first_50 is already truncated
            // So we can safely slice it
            let char_idx = first_50[..last_space].chars().count();
            if char_idx > 20 {
                // Only use word boundary if we keep at least 20 chars
                let prefix: String = first_line.chars().take(char_idx).collect();
                return format!("{}...", prefix.trim_end());
            }
        }

        // Fallback: hard truncate at 47 chars
        let truncated: String = first_line.chars().take(47).collect();
        format!("{}...", truncated)
    }
}
