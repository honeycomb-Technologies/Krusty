use std::collections::HashMap;

/// Expand ${VAR} environment variables, with fallback to credentials store
pub(super) async fn expand_env_var(s: &str, allow_credential_store: bool) -> String {
    let mut result = s.to_string();

    while let Some(start) = result.find("${") {
        if let Some(end_offset) = result[start..].find('}') {
            let end = start + end_offset;
            let var_name = &result[start + 2..end];
            tracing::debug!("Expanding env var: {}", var_name);

            let value = match std::env::var(var_name) {
                Ok(v) => {
                    tracing::debug!("Found {} in environment", var_name);
                    v
                }
                Err(_) => {
                    if allow_credential_store {
                        if let Some(cred_key) = credential_key_for_env(var_name) {
                            tracing::debug!(
                                "Looking up {} in credential store as '{}'",
                                var_name,
                                cred_key
                            );
                            match get_credential(cred_key).await {
                                Some(v) => {
                                    tracing::debug!(
                                        "Found {} in credential store (len={})",
                                        var_name,
                                        v.len()
                                    );
                                    v
                                }
                                None => {
                                    tracing::warn!("Credential '{}' not found in store", cred_key);
                                    String::new()
                                }
                            }
                        } else {
                            tracing::warn!("No credential mapping for {}", var_name);
                            String::new()
                        }
                    } else {
                        tracing::warn!(
                            "Skipping credential-store lookup for {} in untrusted MCP config",
                            var_name
                        );
                        String::new()
                    }
                }
            };

            result.replace_range(start..end + 1, &value);
        } else {
            break;
        }
    }

    result
}

fn credential_key_for_env(env_name: &str) -> Option<&'static str> {
    match env_name {
        "ANTHROPIC_API_KEY" => Some("anthropic"),
        "MINIMAX_API_KEY" => Some("minimax"),
        "OPENROUTER_API_KEY" => Some("openrouter"),
        "OPENAI_API_KEY" => Some("openai"),
        _ => None,
    }
}

async fn get_credential(provider: &str) -> Option<String> {
    let path = crate::paths::config_dir()
        .join("tokens")
        .join("credentials.json");
    tracing::debug!("Looking for credentials at {:?}", path);
    if !path.exists() {
        tracing::warn!("Credentials file not found at {:?}", path);
        return None;
    }
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read credentials: {}", e);
            return None;
        }
    };
    let creds: HashMap<String, String> = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to parse credentials: {}", e);
            return None;
        }
    };
    let result = creds.get(provider).cloned();
    if result.is_some() {
        tracing::debug!("Found credential for '{}'", provider);
    } else {
        let mut available = String::new();
        for key in creds.keys() {
            if !available.is_empty() {
                available.push_str(", ");
            }
            available.push_str(key);
        }
        tracing::warn!(
            "No credential found for '{}' (available: [{}])",
            provider,
            available
        );
    }
    result
}
