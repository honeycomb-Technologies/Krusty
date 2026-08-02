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

    pub fn queue_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).queue_pending_steering(
            session_id,
            pending_id,
            content_json,
        )
    }

    pub fn queue_pending_steering_once(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<bool> {
        super::super::messages::MessageStore::new(&self.db).queue_pending_steering_once(
            session_id,
            pending_id,
            content_json,
        )
    }

    pub fn has_pending_steering(&self, session_id: &str, pending_id: &str) -> Result<bool> {
        super::super::messages::MessageStore::new(&self.db)
            .has_pending_steering(session_id, pending_id)
    }

    pub fn load_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        super::super::messages::MessageStore::new(&self.db)
            .load_pending_steering(session_id, pending_id)
    }

    pub fn promote_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        super::super::messages::MessageStore::new(&self.db)
            .promote_pending_steering(session_id, pending_id)
    }

    pub fn promote_orphaned_pending_steering(&self, session_id: &str) -> Result<usize> {
        super::super::messages::MessageStore::new(&self.db)
            .promote_orphaned_pending_steering(session_id)
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
    /// using the same zero-token, Unicode-safe contract as every client.
    pub fn generate_title_from_content(content: &str) -> String {
        crate::ai::derive_title(content)
    }
}
