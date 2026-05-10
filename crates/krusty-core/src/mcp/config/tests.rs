use super::expansion::expand_env_var;
use super::*;

#[tokio::test]
async fn test_load_without_mcp_json_returns_empty_config() {
    let dir = tempfile::tempdir().unwrap();

    let config = McpConfig::load(dir.path()).await.unwrap();

    assert!(config.mcp_servers.is_empty());
    assert!(config.servers().await.is_empty());
}

#[tokio::test]
async fn test_load_uses_only_project_mcp_json_servers() {
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

    let config = McpConfig::load(dir.path()).await.unwrap();
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
async fn test_expand_env_var() {
    assert_eq!(
        expand_env_var("https://api.example.com").await,
        "https://api.example.com"
    );
}
