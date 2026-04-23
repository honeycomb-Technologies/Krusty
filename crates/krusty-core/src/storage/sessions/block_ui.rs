use anyhow::Result;

use super::SessionManager;

impl SessionManager {
    /// Save block UI state (collapsed, scroll_offset) for a block
    pub fn save_block_ui_state(
        &self,
        session_id: &str,
        block_id: &str,
        collapsed: bool,
        scroll_offset: u16,
    ) -> Result<()> {
        super::super::block_ui::BlockUiStore::new(&self.db).save_block_ui_state(
            session_id,
            block_id,
            collapsed,
            scroll_offset,
        )
    }

    /// Load all block UI states for a session
    pub fn load_block_ui_states(
        &self,
        session_id: &str,
    ) -> Vec<super::super::block_ui::BlockUiState> {
        super::super::block_ui::BlockUiStore::new(&self.db).load_block_ui_states(session_id)
    }
}
