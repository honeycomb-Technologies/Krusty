use super::expansion::expand_env_var;
use super::loader::{
    MAX_MCP_CONFIG_BYTES, MAX_MCP_PACKAGE_FRAGMENTS, MAX_MCP_STARTUP_TIMEOUT_MS,
    MAX_MCP_TOOL_TIMEOUT_MS,
};
use super::*;

async fn load_isolated(working_dir: &std::path::Path) -> McpConfig {
    McpConfig::load_with_global_path(working_dir, &working_dir.join("missing-global.json"))
        .await
        .unwrap()
}

#[tokio::test]
async fn test_load_without_mcp_json_returns_empty_config() {
    let dir = tempfile::tempdir().unwrap();

    let config = load_isolated(dir.path()).await;

    assert!(config.mcp_servers.is_empty());
    assert!(config.servers().await.is_empty());
}

#[tokio::test]
async fn test_load_project_mcp_json_servers() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        r#"{
            "mcpServers": {
                "project-server": {
                    "command": "echo",
                    "args": ["hello"]
                }
            }
        }"#,
    )
    .await
    .unwrap();

    let config = load_isolated(dir.path()).await;
    let servers = config.servers().await;

    assert_eq!(servers.len(), 1);
    assert!(matches!(
        servers.get("project-server"),
        Some(McpServerConfig::Local { command, args, .. })
            if command == "echo" && args.as_slice() == ["hello"]
    ));
    assert!(!servers.contains_key("minimax"));
}

#[tokio::test]
async fn test_parse_local_server() {
    let json = r#"{
        "mcpServers": {
            "minimax": {
                "command": "uvx",
                "args": ["minimax-coding-plan-mcp", "-y"],
                "env": {"MINIMAX_API_KEY": "test"}
            }
        }
    }"#;

    let config: McpConfig = serde_json::from_str(json).unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("minimax"),
        Some(McpServerConfig::Local { .. })
    ));
}

#[tokio::test]
async fn test_parse_remote_server() {
    let json = r#"{
        "mcpServers": {
            "remote": {
                "type": "url",
                "url": "https://mcp.example.com/sse",
                "authorization_token": "token123"
            }
        }
    }"#;

    let config: McpConfig = serde_json::from_str(json).unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("remote"),
        Some(McpServerConfig::Remote { .. })
    ));
}

#[tokio::test]
async fn parses_remote_oauth_pkce_configuration() {
    let config: McpConfig = serde_json::from_str(
        r#"{
            "mcpServers": {
                "remote": {
                    "type": "url",
                    "url": "https://mcp.example.com/mcp",
                    "oauth": {
                        "scopes": ["repo:read", "repo:write"],
                        "clientId": "public-client-id",
                        "clientName": "Krusty Test Client"
                    }
                }
            }
        }"#,
    )
    .unwrap();

    let servers = config.servers().await;
    let McpServerConfig::Remote {
        authorization_token,
        oauth: Some(oauth),
        ..
    } = servers.get("remote").unwrap()
    else {
        panic!("expected OAuth remote server");
    };
    assert!(authorization_token.is_none());
    assert!(oauth.enabled);
    assert_eq!(oauth.scopes, ["repo:read", "repo:write"]);
    assert_eq!(oauth.client_id.as_deref(), Some("public-client-id"));
    assert_eq!(oauth.client_name(), "Krusty Test Client");
}

#[tokio::test]
async fn explicit_bearer_token_can_coexist_as_oauth_override() {
    let config: McpConfig = serde_json::from_str(
        r#"{
            "mcpServers": {
                "remote": {
                    "type": "url",
                    "url": "https://mcp.example.com/mcp",
                    "authorizationToken": "external-token",
                    "oauth": { "scopes": ["read"] }
                }
            }
        }"#,
    )
    .unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("remote"),
        Some(McpServerConfig::Remote {
            authorization_token: Some(token),
            oauth: Some(_),
            ..
        }) if token == "external-token"
    ));
}

#[tokio::test]
async fn test_expand_env_var() {
    assert_eq!(
        expand_env_var("https://api.example.com", false).await,
        "https://api.example.com"
    );
}

#[tokio::test]
async fn untrusted_expansion_never_reads_the_host_environment() {
    let variable = format!("KRUSTY_MCP_SECRET_{}", std::process::id());
    std::env::set_var(&variable, "host-secret");
    let reference = format!("Bearer ${{{variable}}}");

    assert_eq!(expand_env_var(&reference, false).await, reference);

    std::env::remove_var(variable);
}

#[tokio::test]
async fn test_project_local_servers_do_not_auto_connect() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        r#"{
            "mcpServers": {
                "evil": {
                    "command": "sh",
                    "args": ["-c", "echo should-not-run"],
                    "env": {"OPENAI_API_KEY": "${OPENAI_API_KEY}"}
                }
            }
        }"#,
    )
    .await
    .unwrap();

    let config = load_isolated(dir.path()).await;
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("evil"),
        Some(McpServerConfig::Local {
            options: McpServerOptions {
                auto_connect: false,
                source: McpConfigSource::Project,
                ..
            },
            ..
        })
    ));
}

#[tokio::test]
async fn project_remote_servers_do_not_auto_connect_or_inherit_host_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let variable = format!("KRUSTY_MCP_PROJECT_SECRET_{}", std::process::id());
    std::env::set_var(&variable, "host-secret");
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        format!(
            r#"{{
                "mcpServers": {{
                    "remote": {{
                        "type": "url",
                        "url": "https://mcp.example.com/${{{variable}}}",
                        "headers": {{"X-Literal": "${{{variable}}}"}},
                        "envHeaders": {{"X-Secret": "{variable}"}},
                        "bearerTokenEnvVar": "{variable}",
                        "autoConnect": true
                    }}
                }}
            }}"#
        ),
    )
    .await
    .unwrap();

    let config = load_isolated(dir.path()).await;
    let servers = config.servers().await;
    let McpServerConfig::Remote {
        url,
        authorization_token,
        headers,
        options,
        ..
    } = servers.get("remote").unwrap()
    else {
        panic!("expected remote project server");
    };
    assert!(url.contains(&format!("${{{variable}}}")));
    assert_eq!(headers.get("X-Literal"), Some(&format!("${{{variable}}}")));
    assert!(!headers.contains_key("X-Secret"));
    assert!(authorization_token.is_none());
    assert!(!options.auto_connect);
    assert_eq!(options.source, McpConfigSource::Project);
    assert_eq!(options.authority, McpConnectionAuthority::NONE);
    assert!(config.remote_servers_for_api().await.is_empty());

    std::env::remove_var(variable);
}

#[tokio::test]
async fn package_remote_servers_do_not_auto_connect_or_inherit_host_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("package.json");
    let variable = format!("KRUSTY_MCP_PACKAGE_SECRET_{}", std::process::id());
    std::env::set_var(&variable, "host-secret");
    tokio::fs::write(
        &package,
        format!(
            r#"{{"mcpServers":{{"remote":{{"type":"url","url":"https://mcp.example.com","envHeaders":{{"X-Secret":"{variable}"}},"bearerTokenEnvVar":"{variable}","autoConnect":true}}}}}}"#
        ),
    )
    .await
    .unwrap();

    let config = McpConfig::load_with_package_paths(
        dir.path(),
        &dir.path().join("missing-global.json"),
        &[package],
    )
    .await
    .unwrap();
    let servers = config.servers().await;
    let McpServerConfig::Remote {
        authorization_token,
        headers,
        options,
        ..
    } = servers.get("remote").unwrap()
    else {
        panic!("expected remote package server");
    };
    assert!(!headers.contains_key("X-Secret"));
    assert!(authorization_token.is_none());
    assert!(!options.auto_connect);
    assert_eq!(options.source, McpConfigSource::Package);
    assert_eq!(options.authority, McpConnectionAuthority::NONE);

    std::env::remove_var(variable);
}

#[tokio::test]
async fn test_parsed_local_servers_default_to_no_auto_connect() {
    let json = r#"{
        "mcpServers": {
            "local": {
                "command": "echo",
                "args": ["hello"]
            }
        }
    }"#;

    let config: McpConfig = serde_json::from_str(json).unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("local"),
        Some(McpServerConfig::Local {
            options: McpServerOptions {
                auto_connect: false,
                ..
            },
            ..
        })
    ));
}

#[tokio::test]
async fn project_config_overrides_global_by_server_name() {
    let dir = tempfile::tempdir().unwrap();
    let global_path = dir.path().join("global.json");
    tokio::fs::write(
        &global_path,
        r#"{
            "mcpServers": {
                "shared": { "command": "global-command" },
                "global-only": { "command": "global-only" }
            }
        }"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        r#"{
            "mcpServers": {
                "shared": { "command": "project-command" }
            }
        }"#,
    )
    .await
    .unwrap();

    let config = McpConfig::load_with_global_path(dir.path(), &global_path)
        .await
        .unwrap();
    let servers = config.servers().await;

    assert_eq!(servers.len(), 2);
    assert!(matches!(
        servers.get("shared"),
        Some(McpServerConfig::Local { command, options, .. })
            if command == "project-command"
                && options.source == McpConfigSource::Project
                && !options.auto_connect
    ));
    assert!(matches!(
        servers.get("global-only"),
        Some(McpServerConfig::Local { options, .. })
            if options.source == McpConfigSource::Global && options.auto_connect
    ));
}

#[tokio::test]
async fn parses_remote_headers_tokens_timeouts_and_tool_rules() {
    let config: McpConfig = serde_json::from_str(
        r#"{
            "mcpServers": {
                "remote": {
                    "type": "url",
                    "url": "https://mcp.example.com",
                    "enabled": false,
                    "required": true,
                    "startupTimeoutMs": 2500,
                    "toolTimeoutMs": 9000,
                    "headers": {"X-Static": "value"},
                    "tools": {
                        "allow": ["search*"],
                        "deny": ["search_private"],
                        "approval": {"search*": "allow", "search_delete": "prompt"}
                    }
                }
            }
        }"#,
    )
    .unwrap();
    let servers = config.servers().await;
    let server = servers.get("remote").unwrap();

    assert!(!server.is_enabled());
    assert!(server.is_required());
    assert_eq!(server.startup_timeout_ms(), 2500);
    assert_eq!(server.tool_timeout_ms(), 9000);
    assert!(server.allows_tool("search_web"));
    assert!(!server.allows_tool("search_private"));
    assert_eq!(
        server.tool_approval("search_delete"),
        McpToolApproval::Prompt
    );
}

#[tokio::test]
async fn configured_timeouts_are_clamped_to_runtime_safety_limits() {
    let config: McpConfig = serde_json::from_str(&format!(
        r#"{{
            "mcpServers": {{
                "minimum": {{
                    "command": "minimum",
                    "startupTimeoutMs": 0,
                    "toolTimeoutMs": 0
                }},
                "maximum": {{
                    "command": "maximum",
                    "startupTimeoutMs": {},
                    "toolTimeoutMs": {}
                }}
            }}
        }}"#,
        u64::MAX,
        u64::MAX
    ))
    .unwrap();
    let servers = config.servers().await;

    assert_eq!(servers["minimum"].startup_timeout_ms(), 1);
    assert_eq!(servers["minimum"].tool_timeout_ms(), 1);
    assert_eq!(
        servers["maximum"].startup_timeout_ms(),
        MAX_MCP_STARTUP_TIMEOUT_MS
    );
    assert_eq!(
        servers["maximum"].tool_timeout_ms(),
        MAX_MCP_TOOL_TIMEOUT_MS
    );
}

#[tokio::test]
async fn oversized_mcp_config_is_rejected_before_parse() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        vec![b' '; MAX_MCP_CONFIG_BYTES + 1],
    )
    .await
    .unwrap();

    let error =
        McpConfig::load_with_global_path(dir.path(), &dir.path().join("missing-global.json"))
            .await
            .expect_err("oversized MCP config must fail");

    assert!(error.to_string().contains("byte limit"));
}

#[tokio::test]
async fn package_fragment_count_is_bounded_before_file_reads() {
    let dir = tempfile::tempdir().unwrap();
    let fragments = (0..=MAX_MCP_PACKAGE_FRAGMENTS)
        .map(|index| dir.path().join(format!("missing-{index}.json")))
        .collect::<Vec<_>>();

    let error = McpConfig::load_with_package_paths(
        dir.path(),
        &dir.path().join("missing-global.json"),
        &fragments,
    )
    .await
    .expect_err("too many package fragments must fail");

    assert!(error.to_string().contains("package-fragment limit"));
}

#[tokio::test]
async fn package_fragments_are_defaults_below_global_and_project() {
    let dir = tempfile::tempdir().unwrap();
    let package_one = dir.path().join("package-one.json");
    let package_two = dir.path().join("package-two.json");
    let global = dir.path().join("global.json");
    tokio::fs::write(
        &package_one,
        r#"{"mcpServers":{"shared":{"command":"package-one"},"package-only":{"command":"package"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        &package_two,
        r#"{"mcpServers":{"shared":{"command":"package-two"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        &global,
        r#"{"mcpServers":{"shared":{"command":"global"},"global-only":{"command":"global"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"shared":{"command":"project"}}}"#,
    )
    .await
    .unwrap();

    let config =
        McpConfig::load_with_package_paths(dir.path(), &global, &[package_one, package_two])
            .await
            .unwrap();
    let servers = config.servers().await;

    assert!(matches!(
        servers.get("shared"),
        Some(McpServerConfig::Local { command, options, .. })
            if command == "project" && options.source == McpConfigSource::Project
    ));
    assert!(matches!(
        servers.get("global-only"),
        Some(McpServerConfig::Local { options, .. })
            if options.source == McpConfigSource::Global && options.auto_connect
    ));
    assert!(matches!(
        servers.get("package-only"),
        Some(McpServerConfig::Local { options, .. })
            if options.source == McpConfigSource::Package && !options.auto_connect
    ));
}

#[tokio::test]
async fn package_transport_authority_is_exact_and_overrides_do_not_inherit_it() {
    let dir = tempfile::tempdir().unwrap();
    let network_package = dir.path().join("network-package.json");
    let process_package = dir.path().join("process-package.json");
    let global = dir.path().join("global.json");
    tokio::fs::write(
        &network_package,
        r#"{"mcpServers":{"network-local":{"command":"node"},"network-remote":{"type":"url","url":"https://mcp.example/network"},"overridden":{"command":"package"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        &process_package,
        r#"{"mcpServers":{"process-local":{"command":"node"},"process-remote":{"type":"url","url":"https://mcp.example/process"}}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        &global,
        r#"{"mcpServers":{"overridden":{"command":"global"}}}"#,
    )
    .await
    .unwrap();

    let config = McpConfig::load_with_package_configs(
        dir.path(),
        &global,
        &[
            McpPackageConfig::new(network_package, McpConnectionAuthority::new(false, true)),
            McpPackageConfig::new(process_package, McpConnectionAuthority::new(true, false)),
        ],
    )
    .await
    .unwrap();
    let servers = config.servers().await;

    assert!(
        !servers["network-local"].is_authorized_by(servers["network-local"].declared_authority())
    );
    assert!(
        servers["network-remote"].is_authorized_by(servers["network-remote"].declared_authority())
    );
    assert!(
        servers["process-local"].is_authorized_by(servers["process-local"].declared_authority())
    );
    assert!(
        !servers["process-remote"].is_authorized_by(servers["process-remote"].declared_authority())
    );
    assert_eq!(
        servers["overridden"].declared_authority(),
        McpConnectionAuthority::FULL
    );
    assert_eq!(servers["overridden"].source(), McpConfigSource::Global);
}
