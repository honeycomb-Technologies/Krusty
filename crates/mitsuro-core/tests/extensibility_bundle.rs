use std::sync::Arc;

use mitsuro_core::agent::user_hooks::{
    PackageHookConfig, UserHookExecutor, UserHookManager, UserHookResult, UserHookType,
};
use mitsuro_core::extensions::{AgentExtensionManager, ExtensionCallContext};
use mitsuro_core::mcp::{McpConnectionAuthority, McpManager, McpPackageConfig};
use mitsuro_core::plugins::{PluginManager, PluginPermission};
use mitsuro_core::skills::SkillsManager;
use mitsuro_core::tools::{ToolContext, ToolRegistry};
use tempfile::TempDir;

/// Proves that one immutable plugin snapshot can feed every agent-facing
/// extension subsystem through the same installed descriptor.
#[tokio::test]
async fn installed_bundle_activates_skill_extension_mcp_and_hook_contributions() {
    let temp = TempDir::new().expect("temporary directory");
    let package = temp.path().join("package");
    let skill_dir = package.join("skills").join("bundle-skill");
    let extension_dir = package.join("extensions");
    let mcp_dir = package.join("mcp");
    let hooks_dir = package.join("hooks");
    std::fs::create_dir_all(&skill_dir).expect("skill directory");
    std::fs::create_dir_all(&extension_dir).expect("extension directory");
    std::fs::create_dir_all(&mcp_dir).expect("MCP directory");
    std::fs::create_dir_all(&hooks_dir).expect("hooks directory");

    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: bundle-skill
description: Instructions distributed by the end-to-end bundle fixture.
---

# Bundle Skill

Return the fixture marker `bundle-skill-active`.
"#,
    )
    .expect("write skill");
    std::fs::write(
        extension_dir.join("bundle-extension.ts"),
        r#"
export default (mitsuro) => {
  mitsuro.registerTool({
    name: "bundle_echo",
    description: "Echo through an installed bundle",
    parameters: {
      type: "object",
      properties: { value: { type: "string" } },
      required: ["value"]
    },
    execute: ({ value }) => ({ value, source: "installed-bundle" })
  });
  mitsuro.registerCommand("bundle-command", {
    description: "Command contributed by the bundle",
    handler: (argument) => `bundle:${argument}`
  });
};
"#,
    )
    .expect("write extension");
    std::fs::write(
        mcp_dir.join("servers.json"),
        r#"{
  "mcpServers": {
    "bundle-mcp": {
      "type": "url",
      "url": "https://example.invalid/mcp",
      "enabled": false
    }
  }
}"#,
    )
    .expect("write MCP config");
    std::fs::write(
        hooks_dir.join("hooks.json"),
        r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^bundle_echo$",
        "hooks": [
          {
            "type": "command",
            "command": "printf 'bundle-hook-blocked' >&2; exit 2",
            "timeout": 5
          }
        ]
      }
    ]
  }
}"#,
    )
    .expect("write hook config");
    std::fs::write(
        package.join("plugin.toml"),
        r#"
manifest_version = 1
id = "fixture.bundle"
name = "Extensibility Fixture"
version = "1.0.0"
publisher = "mitsuro.tests"
skills = ["skills/bundle-skill"]
agent_extensions = ["extensions/bundle-extension.ts"]
mcp_servers = "mcp/servers.json"
hooks = ["hooks/hooks.json"]

[requested_permissions]
process = true
network = true
"#,
    )
    .expect("write plugin manifest");

    let plugin_manager =
        PluginManager::new(reqwest::Client::new(), temp.path().join("plugin-manager"));
    plugin_manager.ensure_layout().await.expect("plugin layout");
    let installed = plugin_manager
        .install_from_ref(package.to_str().expect("UTF-8 package path"))
        .await
        .expect("install bundle");
    assert_eq!(installed.len(), 1);
    let plugin = &installed[0];
    assert!(plugin.entry_component_path.is_none());

    plugin_manager
        .grant_all_plugin_permissions(&plugin.id)
        .await
        .expect("grant declared permissions");
    plugin_manager
        .ensure_installed_plugin_permission(plugin, PluginPermission::Process)
        .await
        .expect("exact descriptor process grant");

    let mut skills = SkillsManager::new(temp.path().join("global-skills"), None);
    skills.set_package_roots(
        plugin
            .skill_paths
            .iter()
            .cloned()
            .map(|path| (plugin.id.clone(), path))
            .collect(),
    );
    assert!(skills.skill_exists("bundle-skill"));
    assert!(skills
        .load_skill_content("bundle-skill")
        .expect("bundle skill content")
        .contains("bundle-skill-active"));

    let working_dir = temp.path().join("workspace");
    std::fs::create_dir_all(&working_dir).expect("working directory");
    let mcp = McpManager::new(working_dir.clone());
    let permission_status = plugin_manager
        .permission_status_for_installed(plugin)
        .await
        .expect("exact descriptor permission status");
    mcp.set_package_configs(
        plugin
            .mcp_servers_path
            .iter()
            .cloned()
            .map(|path| {
                McpPackageConfig::new(
                    path,
                    McpConnectionAuthority::new(
                        permission_status.granted.process,
                        permission_status.granted.network,
                    ),
                )
            })
            .collect(),
    )
    .await;
    mcp.load_config().await.expect("load package MCP fragment");
    assert!(mcp
        .list_servers()
        .await
        .iter()
        .any(|server| server.name == "bundle-mcp" && !server.enabled));

    let mut hooks = UserHookManager::new();
    let hook_report = hooks
        .replace_package_hooks(
            plugin
                .hook_paths
                .iter()
                .cloned()
                .map(|path| PackageHookConfig::new(&plugin.id, path, &plugin.install_path))
                .collect(),
        )
        .expect("activate package hooks");
    assert_eq!(hook_report.config_count, 1);
    assert_eq!(hook_report.hook_count, 1);
    match UserHookExecutor::execute_matching(
        &mut hooks,
        UserHookType::PreToolUse,
        "bundle_echo",
        &serde_json::json!({"value": "blocked"}),
    )
    .await
    {
        UserHookResult::Block { reason } => assert_eq!(reason, "bundle-hook-blocked"),
        other => panic!("expected installed hook to block, got {other:?}"),
    }

    // Bun is optional for library consumers, but when installed this completes
    // the executable leg of the same package fixture.
    if which::which("bun").is_err() {
        return;
    }
    let registry = Arc::new(ToolRegistry::new());
    let extensions = AgentExtensionManager::new_with_paths(
        &working_dir,
        temp.path().join("extension-runtime"),
        temp.path().join("global-extensions"),
    );
    extensions
        .set_package_roots(plugin.agent_extension_paths.clone())
        .await;
    registry.set_agent_extension_manager(extensions.clone());
    extensions
        .refresh_and_register(&registry)
        .await
        .expect("activate package extension");

    let command = extensions
        .execute_command(
            "bundle-command",
            "ready",
            &ExtensionCallContext::for_turn(
                working_dir.clone(),
                Some(working_dir.clone()),
                Some("bundle-session".to_string()),
                None,
                "supervised",
                false,
            ),
        )
        .await
        .expect("execute bundle command");
    assert_eq!(
        command,
        serde_json::Value::String("bundle:ready".to_string())
    );

    let result = registry
        .execute(
            "bundle_echo",
            serde_json::json!({"value": "ready"}),
            &ToolContext {
                working_dir,
                ..ToolContext::default()
            },
        )
        .await
        .expect("execute bundle tool");
    let envelope: serde_json::Value =
        serde_json::from_str(&result.output).expect("structured tool result");
    assert_eq!(envelope["data"]["value"], "ready");
    assert_eq!(envelope["data"]["source"], "installed-bundle");
}
