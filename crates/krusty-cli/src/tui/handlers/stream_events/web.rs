use crate::tui::app::App;
use crate::tui::blocks::WebSearchBlock;

impl App {
    /// Handle server tool start (web_search, web_fetch)
    pub(super) fn handle_server_tool_start(&mut self, tool_use_id: String, name: String) {
        if name == "web_search" {
            let block = WebSearchBlock::new(tool_use_id, String::new());
            self.runtime.blocks.web_search.push(block);
            self.runtime
                .chat
                .messages
                .push(("web_search".to_string(), String::new()));
        }
    }

    pub(super) fn handle_server_tool_complete(&mut self, id: String, name: String) {
        tracing::info!("Server tool completed: {} ({})", name, id);
    }

    pub(super) fn handle_server_tool_error(&mut self, tool_use_id: String, error_code: String) {
        tracing::warn!("Server tool error: {} ({})", error_code, tool_use_id);
        self.runtime.chat.messages.push((
            "system".to_string(),
            format!("Web tool error: {}", error_code),
        ));
    }

    /// Handle web search results
    pub(super) fn handle_web_search_results(
        &mut self,
        tool_use_id: String,
        results: Vec<crate::ai::types::WebSearchResult>,
    ) {
        tracing::info!(
            "Web search returned {} results ({})",
            results.len(),
            tool_use_id
        );

        if let Some(block) = self
            .runtime
            .blocks
            .web_search
            .iter_mut()
            .find(|block| block.tool_use_id() == tool_use_id)
        {
            block.set_results(results);
        }
    }

    /// Handle web fetch result
    pub(super) fn handle_web_fetch_result(
        &mut self,
        tool_use_id: String,
        content: crate::ai::types::WebFetchContent,
    ) {
        tracing::info!("Web fetch completed: {} ({})", content.url, tool_use_id);
        let title = content.title.as_deref().unwrap_or("page");
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), format!("Fetched: {}", title)));
    }
}
