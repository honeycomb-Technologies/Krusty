//! Skill tool - Invoke skills to get specialized instructions

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::skills::SkillPermission;
use crate::tools::registry::{DelegationPolicy, DelegationSurface, PermissionMode, Tool};
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
        "Load an Agent Skills-compatible instruction package on demand. Compatible global, project, and package roots are discovered automatically. Skill enablement and allow/ask/deny policy are enforced without relaxing inherited tool governance."
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

        let inherited_permission_mode = ctx
            .delegation_policy
            .as_ref()
            .map(|policy| policy.inherited_permission_mode)
            .unwrap_or(ctx.permission_mode);
        let mut governance = json!({
            "surface": DelegationSurface::Skill,
            "permission_mode": inherited_permission_mode,
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
        let Some(skill) = manager.get_skill(&params.skill).cloned() else {
            return ToolResult::error_with_details(
                "skill_not_found",
                format!("Skill '{}' was not discovered", params.skill),
                None,
                Some(governance),
            );
        };
        governance["skill"] = json!({
            "name": &skill.name,
            "source": skill.source,
            "origin": &skill.origin,
            "path": &skill.definition_path,
            "enabled": skill.enabled,
            "permission": skill.permission,
        });

        if !skill.enabled {
            return ToolResult::error_with_details(
                "skill_disabled",
                format!("Skill '{}' is disabled by local policy", params.skill),
                None,
                Some(governance),
            );
        }
        if skill.permission == SkillPermission::Deny {
            return ToolResult::error_with_details(
                "skill_permission_denied",
                format!("Skill '{}' is denied by local policy", params.skill),
                None,
                Some(governance),
            );
        }
        if skill.permission == SkillPermission::Ask
            && inherited_permission_mode != PermissionMode::Supervised
        {
            return ToolResult::error_with_details(
                "skill_approval_required",
                format!(
                    "Skill '{}' has ask policy and requires a supervised parent session or explicit user invocation",
                    params.skill
                ),
                None,
                Some(governance),
            );
        }

        // If a specific file is requested, load that
        if let Some(ref file) = params.file {
            return match manager.load_file_from_skill(&params.skill, file) {
                Ok(content) => ToolResult::success_data_with(
                    json!({
                        "skill": params.skill,
                        "file": file,
                        "content": content,
                        "base_path": &skill.path,
                        "permission": skill.permission,
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
                    "base_path": &skill.path,
                    "definition_path": &skill.definition_path,
                    "origin": &skill.origin,
                    "compatibility": &skill.compatibility,
                    "license": &skill.license,
                    "allowed_tools_advisory": &skill.allowed_tools,
                    "permission": skill.permission,
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
    use crate::skills::{SkillPermission, SkillsManager};
    use crate::tools::registry::{PermissionMode, Tool, ToolContext};
    use std::sync::Arc;
    use tempfile::{tempdir, TempDir};
    use tokio::sync::RwLock;

    fn manager_with_demo_skill() -> (TempDir, Arc<RwLock<SkillsManager>>) {
        let root = tempdir().unwrap();
        let global = root.path().join("global");
        let skill_dir = global.join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: Demo skill\n---\n\nUse this skill.\n",
        )
        .unwrap();
        (
            root,
            Arc::new(RwLock::new(SkillsManager::new(global, None))),
        )
    }

    #[tokio::test]
    async fn skill_tool_returns_governance_metadata() {
        let (_root, manager) = manager_with_demo_skill();
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

    #[tokio::test]
    async fn ask_policy_requires_supervised_parent_but_then_loads() {
        let (_root, manager) = manager_with_demo_skill();
        manager
            .write()
            .await
            .set_skill_permission("demo-skill", SkillPermission::Ask)
            .unwrap();
        let autonomous = ToolContext::default()
            .with_skills_manager(Arc::clone(&manager))
            .with_permission_mode(PermissionMode::Autonomous);
        let rejected = SkillTool
            .execute(serde_json::json!({"skill": "demo-skill"}), &autonomous)
            .await;
        assert!(rejected.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&rejected.output).unwrap();
        assert_eq!(parsed["error"]["code"], "skill_approval_required");

        let supervised = ToolContext::default()
            .with_skills_manager(manager)
            .with_permission_mode(PermissionMode::Supervised);
        let accepted = SkillTool
            .execute(serde_json::json!({"skill": "demo-skill"}), &supervised)
            .await;
        assert!(!accepted.is_error);
    }

    #[tokio::test]
    async fn deny_and_disabled_policies_are_hard_blocks() {
        let (_root, manager) = manager_with_demo_skill();
        manager
            .write()
            .await
            .set_skill_permission("demo-skill", SkillPermission::Deny)
            .unwrap();
        let ctx = ToolContext::default()
            .with_skills_manager(Arc::clone(&manager))
            .with_permission_mode(PermissionMode::Supervised);
        let denied = SkillTool
            .execute(serde_json::json!({"skill": "demo-skill"}), &ctx)
            .await;
        let parsed: serde_json::Value = serde_json::from_str(&denied.output).unwrap();
        assert_eq!(parsed["error"]["code"], "skill_permission_denied");

        manager
            .write()
            .await
            .set_skill_permission("demo-skill", SkillPermission::Allow)
            .unwrap();
        manager
            .write()
            .await
            .set_skill_enabled("demo-skill", false)
            .unwrap();
        let disabled = SkillTool
            .execute(serde_json::json!({"skill": "demo-skill"}), &ctx)
            .await;
        let parsed: serde_json::Value = serde_json::from_str(&disabled.output).unwrap();
        assert_eq!(parsed["error"]["code"], "skill_disabled");
    }
}
