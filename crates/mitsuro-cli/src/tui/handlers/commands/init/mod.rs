mod exploration;
mod template;

pub use template::generate_krab_from_exploration;

use crate::tui::app::{App, View};

impl App {
    /// Handle /init command - intelligently analyze codebase and generate KRAB.md.
    pub(super) fn handle_init_command(&mut self) {
        if !self.is_authenticated() {
            self.generate_basic_krab_template();
            return;
        }

        if self.runtime.channels.init_exploration.is_some() {
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Exploration already in progress...".to_string(),
            ));
            return;
        }

        if self.ui.view == View::StartMenu {
            self.ui.view = View::Chat;
        }

        if self.runtime.current_session_id.is_none() {
            self.create_session("/init - Codebase Analysis");
        }

        self.runtime
            .chat
            .messages
            .push(("user".to_string(), "/init".to_string()));

        let explore_id = format!("init-{}", uuid::Uuid::new_v4());
        let explore_block = crate::tui::blocks::ExploreBlock::with_tool_id(
            "Analyzing codebase for KRAB.md...".to_string(),
            explore_id.clone(),
        );
        self.runtime.blocks.explore.push(explore_block);
        self.runtime
            .chat
            .messages
            .push(("explore".to_string(), explore_id.clone()));
        self.runtime.init_explore_id = Some(explore_id);

        self.start_init_exploration();
    }
}
