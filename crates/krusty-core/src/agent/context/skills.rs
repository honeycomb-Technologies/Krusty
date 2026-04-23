use tokio::sync::RwLock;
use tracing::warn;

use crate::skills::SkillsManager;

/// Build skills context listing available skills.
pub fn build_skills_context(
    skills_manager: &RwLock<SkillsManager>,
    include_project_skills: bool,
) -> String {
    let mut guard = match skills_manager.try_write() {
        Ok(g) => g,
        Err(_) => {
            warn!(
                include_project_skills,
                "Skipping skills context because the skills manager is busy"
            );
            return String::new();
        }
    };

    let skills = if include_project_skills {
        guard.list_skills()
    } else {
        guard.list_global_skills()
    };
    if skills.is_empty() {
        return String::new();
    }

    let mut context =
        String::from("[AVAILABLE SKILLS]\n\nUse the `skill` tool to invoke a skill.\n\n");
    for info in skills {
        context.push_str(&format!("- **{}**: {}\n", info.name, info.description));
        if !info.tags.is_empty() {
            context.push_str(&format!("  Tags: {}\n", info.tags.join(", ")));
        }
    }
    context.push_str("\nTo use: `skill(skill: \"name\")`\n");
    context
}
