use serde::{Deserialize, Serialize};

/// Stable runtime identity for an agent. `creature_name` remains serialized
/// for compatibility, but its value is now a professional display label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub canonical_path: String,
    pub task_name: String,
    pub creature_name: String,
    pub role: String,
    pub ordinal: usize,
}

impl AgentIdentity {
    pub fn root(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            canonical_path: "/root".to_string(),
            task_name: "coordinator".to_string(),
            creature_name: "Agent".to_string(),
            role: "coordinator".to_string(),
            ordinal: 0,
        }
    }

    pub fn child(
        agent_id: impl Into<String>,
        parent_path: &str,
        task_name: impl Into<String>,
        role: impl Into<String>,
        ordinal: usize,
    ) -> Self {
        let task_name = task_name.into();
        let role = role.into();
        let creature_name = format!("Hive Agent {:02}", ordinal + 1);
        let parent_path = parent_path.trim_end_matches('/');
        let path_component = canonical_component(&task_name, ordinal);

        Self {
            agent_id: agent_id.into(),
            canonical_path: format!("{parent_path}/{path_component}"),
            task_name,
            creature_name,
            role,
            ordinal,
        }
    }

    pub fn display_name(&self) -> String {
        format!(
            "{} · {} [{}]",
            self.creature_name, self.task_name, self.role
        )
    }
}

fn canonical_component(task_name: &str, ordinal: usize) -> String {
    let slug = task_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "task" } else { &slug };
    format!("{}-{}", ordinal + 1, slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_identity_is_agent() {
        let root = AgentIdentity::root("root-id");
        assert_eq!(root.creature_name, "Agent");
        assert_eq!(root.canonical_path, "/root");
    }

    #[test]
    fn child_identity_is_deterministic_and_separates_task_from_display_label() {
        let first = AgentIdentity::child("id", "/root", "Honey audit", "reviewer", 0);
        let restored = AgentIdentity::child("id", "/root", "Honey audit", "reviewer", 0);
        assert_eq!(first, restored);
        assert_eq!(first.task_name, "Honey audit");
        assert_eq!(first.creature_name, "Hive Agent 01");
        assert_eq!(
            first.display_name(),
            "Hive Agent 01 · Honey audit [reviewer]"
        );
    }

    #[test]
    fn display_labels_are_unique() {
        let identities = (0..24)
            .map(|ordinal| AgentIdentity::child("id", "/root", "task", "explorer", ordinal))
            .collect::<Vec<_>>();
        let mut names = identities
            .iter()
            .map(|identity| identity.creature_name.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 24);
    }

    #[test]
    fn later_generations_remain_unique_and_stable() {
        let first = AgentIdentity::child("a", "/root", "task", "builder", 0);
        let thirteenth = AgentIdentity::child("b", "/root", "task", "builder", 12);
        assert_eq!(first.creature_name, "Hive Agent 01");
        assert_eq!(thirteenth.creature_name, "Hive Agent 13");
        assert_ne!(first.canonical_path, thirteenth.canonical_path);
    }
}
