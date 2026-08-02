//! Deferred specialist-tool discovery and dispatch.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::ProjectSettings;
use crate::tools::parse_params;
use crate::tools::registry::{tool_policy, Tool, ToolCategory, ToolContext, ToolResult};

const MAX_SEARCH_RESULTS: usize = 12;
const MAX_GUIDANCE_CHARS: usize = 8 * 1024;

const NON_DEFERRED_TOOLS: &[&str] = &[
    "AskUserQuestion",
    "add_subtask",
    "agent",
    "autonomous_task",
    "enter_plan_mode",
    "send_user_message",
    "set_dependency",
    "set_work_mode",
    "sleep",
    "task_complete",
    "task_start",
    "tool_search",
    "workflow_propose",
    "workflow_update",
];

pub struct ToolSearchTool;

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search, inspect, or execute tools not present in the direct tool list; target permissions still apply."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            "Use search to discover a recommended tool that is not directly available, describe to inspect its schema, and execute to call it. When a policy error recommends a hidden specialist, route through this tool instead of retrying the blocked command.",
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "describe", "execute"]
                },
                "query": {
                    "type": "string",
                    "description": "Search keywords"
                },
                "tool": {
                    "type": "string",
                    "description": "Exact target name"
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for execute"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(params) => params,
            Err(error) => return error,
        };

        let Some(registry) = ctx.tool_registry.as_ref() else {
            return ToolResult::error_with_code(
                "tool_unavailable",
                "Deferred tool registry is not available in this runtime",
            );
        };
        let disabled = project_disabled_tools(ctx);

        match params.action.as_str() {
            "search" => {
                let query = params.query.as_deref().unwrap_or_default();
                let mut matches = registry
                    .get_ai_tools_all()
                    .await
                    .into_iter()
                    .filter(|tool| is_deferred_tool(&tool.name))
                    .filter(|tool| !disabled.iter().any(|name| name == &tool.name))
                    .filter(|tool| delegated_tool_allowed(ctx, &tool.name, &Value::Null))
                    .filter_map(|tool| {
                        relevance_score(query, &tool.name, &tool.description)
                            .map(|score| (score, tool))
                    })
                    .collect::<Vec<_>>();
                matches.sort_by(|(left_score, left), (right_score, right)| {
                    right_score
                        .cmp(left_score)
                        .then_with(|| left.name.cmp(&right.name))
                });

                let tools = matches
                    .into_iter()
                    .take(MAX_SEARCH_RESULTS)
                    .map(|(_, tool)| {
                        let policy = tool_policy(&tool.name);
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "category": category_name(policy.category),
                            "requires_supervised_approval": policy.requires_supervised_approval,
                            "allowed_in_plan_mode": policy.allowed_in_plan_mode,
                        })
                    })
                    .collect::<Vec<_>>();
                let count = tools.len();

                ToolResult::success_data(json!({
                    "query": query,
                    "tools": tools,
                    "count": count,
                }))
            }
            "describe" => {
                let target = match required_target(&params, &disabled) {
                    Ok(target) => target,
                    Err(error) => return error,
                };
                let Some(tool) = registry.get(&target).await else {
                    return ToolResult::error_with_code(
                        "tool_not_found",
                        format!("Deferred tool '{target}' is not registered"),
                    );
                };
                if let Err(error) = enforce_delegated_policy(ctx, &target, &Value::Null) {
                    return error;
                }
                let policy = tool_policy(&target);
                let mut description = json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.parameters_schema(),
                    "category": category_name(policy.category),
                    "requires_supervised_approval": policy.requires_supervised_approval,
                    "allowed_in_plan_mode": policy.allowed_in_plan_mode,
                });
                if let Some(guidance) = tool.prompt().map(str::trim).filter(|text| !text.is_empty())
                {
                    description["guidance"] =
                        Value::String(truncate_guidance(guidance, MAX_GUIDANCE_CHARS));
                }
                ToolResult::success_data(description)
            }
            "execute" => {
                let target = match required_target(&params, &disabled) {
                    Ok(target) => target,
                    Err(error) => return error,
                };
                let arguments = params.arguments.unwrap_or_else(|| json!({}));
                if let Err(error) = enforce_delegated_policy(ctx, &target, &arguments) {
                    return error;
                }
                registry
                    .execute(&target, arguments, ctx)
                    .await
                    .unwrap_or_else(|| {
                        ToolResult::error_with_code(
                            "tool_not_found",
                            format!("Deferred tool '{target}' is not registered"),
                        )
                    })
            }
            _ => ToolResult::invalid_parameters(
                "Unknown action. Use 'search', 'describe', or 'execute'",
            ),
        }
    }
}

fn required_target(params: &Params, disabled: &[String]) -> Result<String, ToolResult> {
    let Some(target) = params
        .tool
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Err(ToolResult::invalid_parameters(
            "tool is required for describe and execute",
        ));
    };
    if !is_deferred_tool(target) {
        return Err(ToolResult::error_with_code(
            "tool_not_deferred",
            format!("Tool '{target}' is not available through deferred dispatch"),
        ));
    }
    if disabled.iter().any(|name| name == target) {
        return Err(ToolResult::error_with_code(
            "disabled_by_project",
            format!("Tool '{target}' is disabled in .mitsuro/settings.json"),
        ));
    }
    Ok(target.to_string())
}

fn project_disabled_tools(ctx: &ToolContext) -> Vec<String> {
    let project_dir = ctx.project_dir.as_deref().unwrap_or(&ctx.working_dir);
    ProjectSettings::load(project_dir)
        .disabled_tools
        .unwrap_or_default()
}

fn delegated_tool_allowed(ctx: &ToolContext, name: &str, arguments: &Value) -> bool {
    ctx.delegation_policy.as_ref().is_none_or(|policy| {
        policy
            .authorize_tool_call(name, arguments, ctx.plan_mode)
            .is_ok()
    })
}

fn enforce_delegated_policy(
    ctx: &ToolContext,
    name: &str,
    arguments: &Value,
) -> Result<(), ToolResult> {
    let Some(policy) = ctx.delegation_policy.as_ref() else {
        return Ok(());
    };
    policy
        .authorize_tool_call(name, arguments, ctx.plan_mode)
        .map_err(|reason| {
            ToolResult::error_with_details(
                "delegated_policy_block",
                reason,
                None,
                Some(json!({
                    "tool": name,
                    "delegation_policy": policy.audit_json(),
                })),
            )
        })
}

fn is_deferred_tool(name: &str) -> bool {
    !NON_DEFERRED_TOOLS.contains(&name)
}

fn relevance_score(query: &str, name: &str, description: &str) -> Option<usize> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Some(1);
    }

    let name = name.to_ascii_lowercase();
    let description = description.to_ascii_lowercase();
    let terms = query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let mut score = usize::from(name == query) * 100 + usize::from(name.contains(&query)) * 20;
    for term in terms {
        score += usize::from(name.contains(term)) * 8;
        score += usize::from(description.contains(term)) * 2;
    }
    (score > 0).then_some(score)
}

fn category_name(category: ToolCategory) -> &'static str {
    match category {
        ToolCategory::ReadOnly => "read_only",
        ToolCategory::Write => "write",
        ToolCategory::Interactive => "interactive",
    }
}

fn truncate_guidance(guidance: &str, max_chars: usize) -> String {
    if guidance.chars().count() <= max_chars {
        return guidance.to_string();
    }

    let mut truncated = guidance.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n[GUIDANCE TRUNCATED]");
    truncated
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    use super::ToolSearchTool;
    use crate::skills::SkillsManager;
    use crate::tools::registry::{
        DelegationPolicy, PermissionMode, Tool, ToolContext, ToolRegistry, ToolResult,
    };
    use crate::tools::SkillTool;

    #[test]
    fn report_and_skill_remain_reachable_as_deferred_tools() {
        assert!(super::is_deferred_tool("report"));
        assert!(super::is_deferred_tool("skill"));
        assert!(!super::is_deferred_tool("tool_search"));
    }

    struct SpecialistTool;

    #[async_trait]
    impl Tool for SpecialistTool {
        fn name(&self) -> &str {
            "specialist_demo"
        }

        fn description(&self) -> &str {
            "Inspect database migration state"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "additionalProperties": false})
        }

        fn prompt(&self) -> Option<&str> {
            Some("Inspect migrations without changing them.")
        }

        async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success_data(json!({"executed": true}))
        }
    }

    async fn context_with_registry() -> (Arc<ToolRegistry>, ToolContext) {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(ToolSearchTool)).await;
        registry.register(Arc::new(SpecialistTool)).await;
        let context = ToolContext {
            working_dir: PathBuf::from("/tmp"),
            ..Default::default()
        }
        .with_tool_registry(Arc::clone(&registry));
        (registry, context)
    }

    #[tokio::test]
    async fn searches_and_executes_specialists_through_the_registry() {
        let (registry, context) = context_with_registry().await;
        let search = registry
            .execute(
                "tool_search",
                json!({"action": "search", "query": "migration"}),
                &context,
            )
            .await
            .unwrap();
        assert!(search.output.contains("specialist_demo"));

        let describe = registry
            .execute(
                "tool_search",
                json!({"action": "describe", "tool": "specialist_demo"}),
                &context,
            )
            .await
            .unwrap();
        assert!(describe
            .output
            .contains("Inspect migrations without changing them."));

        let execute = registry
            .execute(
                "tool_search",
                json!({
                    "action": "execute",
                    "tool": "specialist_demo",
                    "arguments": {}
                }),
                &context,
            )
            .await
            .unwrap();
        assert!(!execute.is_error);
        assert!(execute.output.contains("executed"));
    }

    #[tokio::test]
    async fn refuses_project_disabled_specialists() {
        let temp = tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".mitsuro")).unwrap();
        std::fs::write(
            temp.path().join(".mitsuro/settings.json"),
            r#"{"disabled_tools":["specialist_demo"]}"#,
        )
        .unwrap();

        let (registry, mut context) = context_with_registry().await;
        context.working_dir = temp.path().to_path_buf();
        context.project_dir = Some(temp.path().to_path_buf());
        let result = registry
            .execute(
                "tool_search",
                json!({
                    "action": "execute",
                    "tool": "specialist_demo",
                    "arguments": {}
                }),
                &context,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("disabled_by_project"));
    }

    #[tokio::test]
    async fn delegated_read_only_policy_cannot_dispatch_write_specialists() {
        let (registry, context) = context_with_registry().await;
        let context = context.with_delegation_policy(DelegationPolicy::for_subagent_explore(
            PermissionMode::Autonomous,
            Some(5),
        ));
        let result = registry
            .execute(
                "tool_search",
                json!({
                    "action": "execute",
                    "tool": "specialist_demo",
                    "arguments": {}
                }),
                &context,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("delegated_policy_block"));
    }

    #[tokio::test]
    async fn deferred_skill_execution_uses_the_runtime_skills_manager() {
        let global = tempdir().unwrap();
        let project = tempdir().unwrap();
        let skill_dir = global.path().join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: Demo\n---\n\nUse the demo workflow.\n",
        )
        .unwrap();

        let manager = Arc::new(RwLock::new(SkillsManager::new(
            global.path().to_path_buf(),
            Some(project.path().to_path_buf()),
        )));
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(ToolSearchTool)).await;
        registry.register(Arc::new(SkillTool)).await;
        let context = ToolContext {
            working_dir: project.path().to_path_buf(),
            project_dir: Some(project.path().to_path_buf()),
            ..Default::default()
        }
        .with_tool_registry(Arc::clone(&registry))
        .with_skills_manager(manager);

        let result = registry
            .execute(
                "tool_search",
                json!({
                    "action": "execute",
                    "tool": "skill",
                    "arguments": {"skill": "demo-skill"}
                }),
                &context,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("Use the demo workflow"));
    }
}
