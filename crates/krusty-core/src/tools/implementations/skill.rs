//! Skill tool - Invoke skills to get specialized instructions

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::registry::{DelegationPolicy, DelegationSurface, Tool};
use crate::tools::{parse_params, ToolContext, ToolResult};

pub struct SkillTool;

#[derive(Deserialize)]
struct Params {
    /// Name of the skill to invoke
    skill: String,
    /// Optional: specific file within the skill to read
    #[serde(default)]
    file: Option<String>,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Invoke a skill to get specialized instructions and guidance. Skills are loaded from ~/.config/krusty/skills/ or .krusty/skills/ in the project. Use this when a task matches an available skill's description. Returns error if skill not found - check skill name spelling."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The name of the skill to invoke (e.g., 'git-commit', 'code-review')"
                },
                "file": {
                    "type": "string",
                    "description": "Optional: specific file within the skill to read (e.g., 'CHECKLIST.md')"
                }
            },
            "required": ["skill"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let governance = json!({
            "surface": DelegationSurface::Skill,
            "permission_mode": ctx.permission_mode,
            "delegation_policy": ctx
                .delegation_policy
                .as_ref()
                .map(DelegationPolicy::audit_json),
        });

        let Some(skills_manager) = &ctx.skills_manager else {
            return ToolResult::error_with_details(
                "tool_unavailable",
                "Skills manager not available",
                None,
                Some(governance),
            );
        };

        let mut manager = skills_manager.write().await;

        // If a specific file is requested, load that
        if let Some(ref file) = params.file {
            return match manager.load_file_from_skill(&params.skill, file) {
                Ok(content) => ToolResult::success_data_with(
                    json!({
                        "skill": params.skill,
                        "file": file,
                        "content": content,
                    }),
                    Vec::new(),
                    None,
                    Some(governance),
                ),
                Err(e) => ToolResult::error_with_details(
                    "skill_file_load_failed",
                    format!("Failed to load {}: {}", file, e),
                    None,
                    Some(governance),
                ),
            };
        }

        // Load the main SKILL.md content
        match manager.load_skill_content(&params.skill) {
            Ok(content) => ToolResult::success_data_with(
                json!({
                    "skill": params.skill,
                    "content": content,
                }),
                Vec::new(),
                None,
                Some(governance),
            ),
            Err(e) => ToolResult::error_with_details(
                "skill_not_found",
                format!("Skill '{}' not found: {}", params.skill, e),
                None,
                Some(governance),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SkillTool;
    use crate::skills::SkillsManager;
    use crate::tools::registry::{PermissionMode, Tool, ToolContext};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn skill_tool_returns_governance_metadata() {
        let global = tempdir().unwrap();
        let project = tempdir().unwrap();
        let skill_dir = global.path().join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: Demo skill\n---\n\nUse this skill.\n",
        )
        .unwrap();

        let manager = Arc::new(RwLock::new(SkillsManager::new(
            global.path().to_path_buf(),
            Some(project.path().to_path_buf()),
        )));
        let ctx = ToolContext::default()
            .with_skills_manager(manager)
            .with_permission_mode(PermissionMode::Autonomous);

        let result = SkillTool
            .execute(serde_json::json!({"skill": "demo-skill"}), &ctx)
            .await;

        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["metadata"]["surface"], "skill");
        assert_eq!(parsed["metadata"]["permission_mode"], "autonomous");
        assert_eq!(parsed["data"]["skill"], "demo-skill");
    }
}
