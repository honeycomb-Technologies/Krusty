//! Explicit `/skill:<name> [request]` invocation.

use krusty_core::skills::SkillPermission;

use crate::tui::app::App;

impl App {
    pub(super) fn handle_explicit_skill_command(&mut self, command: &str) {
        let (selector, request) = command
            .split_once(char::is_whitespace)
            .map(|(selector, request)| (selector, request.trim()))
            .unwrap_or((command, ""));
        // The dispatcher already matched this prefix case-insensitively.
        let name = selector.get("/skill:".len()..).unwrap_or_default().trim();
        if name.is_empty() {
            self.skill_command_error("Usage: /skill:<name> [request]");
            return;
        }

        let loaded = (|| -> Result<_, String> {
            let mut manager = self
                .services
                .skills_manager
                .try_write()
                .map_err(|_| "Skills manager is busy; try again.".to_string())?;
            let skill = manager.get_skill(name).cloned().ok_or_else(|| {
                format!("Skill '{name}' was not found. Use /skills to browse available skills.")
            })?;
            if !skill.enabled {
                return Err(format!(
                    "Skill '{name}' is disabled. Enable it from /skills first."
                ));
            }
            if skill.permission == SkillPermission::Deny {
                return Err(format!(
                    "Skill '{name}' is denied by local policy. Change it from /skills first."
                ));
            }
            let content = manager
                .load_skill_content_for_user(name)
                .map_err(|error| error.to_string())?;
            Ok((skill, content))
        })();
        let (skill, content) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                self.skill_command_error(error);
                return;
            }
        };

        // A slash command is an explicit user action, so `ask` policy is
        // satisfied here. The content does not grant tool permission: the agent
        // still executes under the session's inherited governance mode.
        let request = if request.is_empty() {
            "Apply these instructions and ask me what I want to do with this skill."
        } else {
            request
        };
        let prompt = format!(
            "The user explicitly invoked the Agent Skill `{}`. Follow its instructions for the request below. Relative resource paths resolve from `{}`. This skill does not override tool permissions.\n\n<skill_instructions name=\"{}\">\n{}\n</skill_instructions>\n\nUser request: {}",
            skill.name,
            skill.path.display(),
            skill.name,
            content,
            request
        );
        self.handle_input_submit(prompt);
    }

    fn skill_command_error(&mut self, message: impl Into<String>) {
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message.into()));
    }
}
