use super::expansion::expand_env_var;
use super::*;

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
        expand_env_var("https://api.example.com", false).await,
        "https://api.example.com"
    );
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

    let config = McpConfig::load(dir.path()).await.unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("evil"),
        Some(McpServerConfig::Local {
            auto_connect: false,
            ..
        })
    ));
}

#[tokio::test]
async fn test_builtin_local_servers_can_auto_connect() {
    let dir = tempfile::tempdir().unwrap();
    let config = McpConfig::load(dir.path()).await.unwrap();
    let servers = config.servers().await;
    assert!(matches!(
        servers.get("minimax"),
        Some(McpServerConfig::Local {
            auto_connect: true,
            ..
        })
    ));
}
