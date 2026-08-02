use tokio::sync::RwLock;
use tracing::warn;

use crate::skills::SkillsManager;

use super::{truncate_utf8, truncate_utf8_bytes};

const MAX_SKILL_ITEMS: usize = 24;
const MAX_SKILLS_CONTEXT_BYTES: usize = 6 * 1024;
const MAX_SKILL_DESCRIPTION_CHARS: usize = 160;
const MAX_SKILL_TAGS: usize = 6;

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

    let skills = guard.list_model_skills(include_project_skills);
    if skills.is_empty() {
        return String::new();
    }

    let total_skills = skills.len();
    let mut context = String::from(
        "[AVAILABLE SKILLS]\n\nSkills follow the Agent Skills standard and are loaded on demand. Invoke one through `tool_search` with action=execute, tool=skill, and arguments={\"skill\":\"name\"}. Loading instructions never relaxes the parent tool permission policy.\n\n",
    );
    for info in skills.into_iter().take(MAX_SKILL_ITEMS) {
        context.push_str(&format!(
            "- **{}** ({}; {}): {}\n",
            info.name,
            info.origin,
            info.permission,
            truncate_utf8(&info.description, MAX_SKILL_DESCRIPTION_CHARS)
        ));
        if !info.tags.is_empty() {
            context.push_str(&format!(
                "  Tags: {}\n",
                info.tags
                    .iter()
                    .take(MAX_SKILL_TAGS)
                    .map(|tag| truncate_utf8(tag, 60))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if total_skills > MAX_SKILL_ITEMS {
        context.push_str(&format!(
            "- ... {} more skills omitted from prompt; invoke a known skill by name.\n",
            total_skills - MAX_SKILL_ITEMS
        ));
    }
    context.push_str(
        "\nPolicies: allow loads normally; ask is available only in a supervised parent session. Denied, disabled, and user-only skills are not advertised.\nTo use: `tool_search(action: \"execute\", tool: \"skill\", arguments: {\"skill\": \"name\"})`\n",
    );

    if context.len() <= MAX_SKILLS_CONTEXT_BYTES {
        return context;
    }

    const MARKER: &str = "\n[SKILL CATALOG TRUNCATED AT REQUEST BUDGET]";
    let mut bounded = truncate_utf8_bytes(&context, MAX_SKILLS_CONTEXT_BYTES - MARKER.len());
    bounded.push_str(MARKER);
    bounded
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use super::{build_skills_context, MAX_SKILLS_CONTEXT_BYTES};
    use crate::skills::{SkillPermission, SkillsManager};

    #[test]
    fn skills_context_is_bounded_and_limits_catalog_entries() {
        let temp = TempDir::new().expect("temp dir");
        let global = temp.path().join("skills");
        fs::create_dir_all(&global).expect("skills dir");
        for index in 0..40 {
            let dir = global.join(format!("skill-{index:02}"));
            fs::create_dir_all(&dir).expect("skill dir");
            fs::write(
                dir.join("SKILL.md"),
                format!(
                    "---\nname: skill-{index:02}\ndescription: {}\n---\nbody",
                    "long discovery description ".repeat(30)
                ),
            )
            .expect("skill file");
        }

        let manager = RwLock::new(SkillsManager::new(global, None));
        let context = build_skills_context(&manager, false);

        assert!(context.len() <= MAX_SKILLS_CONTEXT_BYTES);
        assert!(context.contains("more skills omitted"));
        assert!(!context.contains("skill-39"));
    }

    #[test]
    fn skills_context_excludes_disabled_denied_and_user_only_skills() {
        let temp = TempDir::new().expect("temp dir");
        let global = temp.path().join("skills");
        for (name, extra) in [
            ("visible-skill", ""),
            ("disabled-skill", ""),
            ("denied-skill", ""),
            ("user-only-skill", "disable-model-invocation: true\n"),
        ] {
            let dir = global.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} description\n{extra}---\nbody"),
            )
            .unwrap();
        }
        let mut manager = SkillsManager::new(global, None);
        manager.set_skill_enabled("disabled-skill", false).unwrap();
        manager
            .set_skill_permission("denied-skill", SkillPermission::Deny)
            .unwrap();
        let context = build_skills_context(&RwLock::new(manager), true);

        assert!(context.contains("visible-skill"));
        assert!(!context.contains("disabled-skill"));
        assert!(!context.contains("denied-skill"));
        assert!(!context.contains("user-only-skill"));
    }
}
