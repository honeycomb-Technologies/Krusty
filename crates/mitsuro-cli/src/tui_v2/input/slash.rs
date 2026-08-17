//! Typed slash-command parsing for the conversation composer.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    /// `/home`, `/new`, and `/clear` — Home screen with a fresh draft session.
    NewConversation,
    Sessions,
    Model,
    Fast,
    Connections,
    InitializeProject,
    Appearance,
    Compact,
    Help,
    Processes,
    Extensions,
    PlanGoal,
    Permissions,
    Update,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SlashInput<'a> {
    NotCommand,
    Known {
        command: SlashCommand,
        arguments: &'a str,
    },
    Unknown {
        name: &'a str,
        arguments: &'a str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlashDefinition {
    pub primary: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}

pub const DEFINITIONS: &[SlashDefinition] = &[
    SlashDefinition {
        primary: "/home",
        aliases: &["/new", "/clear"],
        description: "Home - start a fresh conversation",
    },
    SlashDefinition {
        primary: "/load",
        aliases: &[],
        description: "Open a conversation",
    },
    SlashDefinition {
        primary: "/model",
        aliases: &[],
        description: "Choose the exact model",
    },
    SlashDefinition {
        primary: "/fast",
        aliases: &[],
        description: "Toggle fast mode",
    },
    SlashDefinition {
        primary: "/auth",
        aliases: &[],
        description: "Manage connections",
    },
    SlashDefinition {
        primary: "/init",
        aliases: &[],
        description: "Create or improve KRAB.md",
    },
    SlashDefinition {
        primary: "/theme",
        aliases: &[],
        description: "Theme and motion",
    },
    SlashDefinition {
        primary: "/pinch",
        aliases: &["/compact"],
        description: "Compact this conversation",
    },
    SlashDefinition {
        primary: "/cmd",
        aliases: &["/help"],
        description: "Show all controls",
    },
    SlashDefinition {
        primary: "/ps",
        aliases: &["/processes"],
        description: "Inspect background processes",
    },
    SlashDefinition {
        primary: "/extensions",
        aliases: &["/skills", "/plugins", "/mcp", "/hooks"],
        description: "Manage extensions and services",
    },
    SlashDefinition {
        primary: "/plan",
        aliases: &["/goal"],
        description: "Inspect the plan and Goal",
    },
    SlashDefinition {
        primary: "/permissions",
        aliases: &["/perm"],
        description: "Toggle permission mode",
    },
    SlashDefinition {
        primary: "/update",
        aliases: &[],
        description: "Install the available Mitsuro update",
    },
];

pub fn suggestions(input: &str) -> Vec<&'static SlashDefinition> {
    let query = input.trim();
    if !query.starts_with('/')
        || query.contains(char::is_whitespace)
        || query.get(1..).is_some_and(|value| value.contains('/'))
    {
        return Vec::new();
    }
    let query = query.to_ascii_lowercase();
    DEFINITIONS
        .iter()
        .filter(|definition| {
            definition.primary.starts_with(&query)
                || definition
                    .aliases
                    .iter()
                    .any(|alias| alias.starts_with(&query))
        })
        .collect()
}

pub fn parse(input: &str) -> SlashInput<'_> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') || trimmed.contains('\n') {
        return SlashInput::NotCommand;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    let arguments = parts.next().unwrap_or_default().trim();
    if name.get(1..).is_some_and(|name| name.contains('/')) {
        return SlashInput::NotCommand;
    }
    let command = match name.to_ascii_lowercase().as_str() {
        // Home / new / clear are the same product action: leave the active
        // session draft and land on Home ready for a fresh conversation.
        "/home" | "/new" | "/clear" => SlashCommand::NewConversation,
        "/load" => SlashCommand::Sessions,
        "/model" => SlashCommand::Model,
        "/fast" => SlashCommand::Fast,
        "/auth" => SlashCommand::Connections,
        "/init" => SlashCommand::InitializeProject,
        "/theme" => SlashCommand::Appearance,
        "/pinch" | "/compact" => SlashCommand::Compact,
        "/cmd" | "/help" => SlashCommand::Help,
        "/ps" | "/processes" => SlashCommand::Processes,
        "/skills" | "/plugins" | "/extensions" | "/mcp" | "/hooks" => SlashCommand::Extensions,
        "/plan" | "/goal" => SlashCommand::PlanGoal,
        "/permissions" | "/perm" => SlashCommand::Permissions,
        "/update" => SlashCommand::Update,
        _ => {
            return SlashInput::Unknown { name, arguments };
        }
    };
    SlashInput::Known { command, arguments }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_one_typed_command() {
        assert_eq!(
            parse("/compact preserve the current task"),
            SlashInput::Known {
                command: SlashCommand::Compact,
                arguments: "preserve the current task",
            }
        );
        for name in ["/home", "/new", "/clear"] {
            assert_eq!(
                parse(name),
                SlashInput::Known {
                    command: SlashCommand::NewConversation,
                    arguments: "",
                },
                "{name} must start a fresh home conversation"
            );
        }
    }

    #[test]
    fn paths_multiline_text_and_unknown_commands_are_not_misclassified() {
        assert_eq!(parse("src/main.rs"), SlashInput::NotCommand);
        assert_eq!(parse("/tmp/project"), SlashInput::NotCommand);
        assert_eq!(parse("/model\nthen continue"), SlashInput::NotCommand);
        assert_eq!(
            parse("/release candidate"),
            SlashInput::Unknown {
                name: "/release",
                arguments: "candidate",
            }
        );
    }

    #[test]
    fn slash_opens_the_complete_scrollable_command_catalog() {
        let all = suggestions("/");
        assert_eq!(all.len(), DEFINITIONS.len());
        assert!(all.iter().any(|command| command.primary == "/permissions"));
        assert!(all.iter().any(|command| command.primary == "/update"));
        assert_eq!(suggestions("/plugins")[0].primary, "/extensions");
        assert!(suggestions("/tmp/project").is_empty());
        assert!(suggestions("/model now").is_empty());
    }
}
