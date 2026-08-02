use std::{collections::HashSet, fs, path::Path};

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::model::{PackageHookConfig, UserHook, UserHookType};

const MAX_HOOK_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_PACKAGE_HOOK_CONFIGS: usize = 256;
const MAX_PACKAGE_HOOKS: usize = 4096;
const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_PATTERN_BYTES: usize = 8 * 1024;
const MAX_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug)]
struct ParsedHook {
    hook_type: UserHookType,
    tool_pattern: String,
    command: String,
    enabled: bool,
    timeout_seconds: u64,
}

pub(super) fn load_package_hooks(configs: &[PackageHookConfig]) -> Result<Vec<UserHook>> {
    if configs.len() > MAX_PACKAGE_HOOK_CONFIGS {
        bail!(
            "package hook replacement declares {} configs; maximum is {}",
            configs.len(),
            MAX_PACKAGE_HOOK_CONFIGS
        );
    }

    let mut seen_configs = HashSet::new();
    let mut hooks = Vec::new();
    for config in configs {
        if config.plugin_id.trim().is_empty() {
            bail!("package hook plugin id cannot be empty");
        }

        let package_root = fs::canonicalize(&config.package_root).with_context(|| {
            format!(
                "failed to resolve package root {} for plugin '{}'",
                config.package_root.display(),
                config.plugin_id
            )
        })?;
        if !fs::metadata(&package_root)?.is_dir() {
            bail!(
                "package hook root {} for plugin '{}' must be a directory",
                package_root.display(),
                config.plugin_id
            );
        }
        let config_path = fs::canonicalize(&config.config_path).with_context(|| {
            format!(
                "failed to resolve hook config {} for plugin '{}'",
                config.config_path.display(),
                config.plugin_id
            )
        })?;
        if !config_path.starts_with(&package_root) {
            bail!(
                "hook config {} escapes package root {} for plugin '{}'",
                config_path.display(),
                package_root.display(),
                config.plugin_id
            );
        }
        if !seen_configs.insert(config_path.clone()) {
            bail!(
                "hook config {} is declared more than once",
                config_path.display()
            );
        }

        let metadata = fs::metadata(&config_path)
            .with_context(|| format!("failed to inspect hook config {}", config_path.display()))?;
        if !metadata.is_file() {
            bail!(
                "hook config {} must be a regular file",
                config_path.display()
            );
        }
        if metadata.len() > MAX_HOOK_CONFIG_BYTES {
            bail!(
                "hook config {} is {} bytes; maximum is {}",
                config_path.display(),
                metadata.len(),
                MAX_HOOK_CONFIG_BYTES
            );
        }

        let bytes = fs::read(&config_path)
            .with_context(|| format!("failed to read hook config {}", config_path.display()))?;
        let document = parse_document(&bytes, &config_path)?;
        let parsed = parse_document_hooks(&document).with_context(|| {
            format!(
                "invalid hook config {} for plugin '{}'",
                config_path.display(),
                config.plugin_id
            )
        })?;

        for parsed_hook in parsed {
            if hooks.len() >= MAX_PACKAGE_HOOKS {
                bail!(
                    "package hooks exceed the global maximum of {}",
                    MAX_PACKAGE_HOOKS
                );
            }
            let id = stable_hook_id(&config.plugin_id, &config_path, hooks.len(), &parsed_hook);
            hooks.push(UserHook::new_package(
                id,
                parsed_hook.hook_type,
                parsed_hook.tool_pattern,
                parsed_hook.command,
                parsed_hook.enabled,
                parsed_hook.timeout_seconds,
                config.plugin_id.clone(),
                config_path.clone(),
                package_root.clone(),
            ));
        }
    }

    Ok(hooks)
}

fn parse_document(bytes: &[u8], path: &Path) -> Result<Value> {
    if let Ok(value) = serde_json::from_slice(bytes) {
        return Ok(value);
    }

    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
    let toml_value = toml::from_str::<toml::Value>(text)
        .with_context(|| format!("{} is neither valid JSON nor valid TOML", path.display()))?;
    serde_json::to_value(toml_value).context("failed to normalize TOML hook config")
}

fn parse_document_hooks(document: &Value) -> Result<Vec<ParsedHook>> {
    match document {
        Value::Array(items) => parse_flat_hooks(items),
        Value::Object(object) => {
            if looks_like_flat_hook(object) {
                return parse_flat_hook(object, None, None, None)
                    .map(|hook| hook.into_iter().collect());
            }

            if let Some(hooks) = object.get("hooks") {
                return match hooks {
                    Value::Array(items) => parse_flat_hooks(items),
                    Value::Object(events) => parse_event_map(events),
                    _ => bail!("'hooks' must be an array or event object"),
                };
            }

            if object.keys().any(|name| parse_hook_type(name).is_some()) {
                return parse_event_map(object);
            }

            bail!("expected a hook object, hook array, or an object containing 'hooks'")
        }
        _ => bail!("hook config root must be an object or array"),
    }
}

fn parse_flat_hooks(items: &[Value]) -> Result<Vec<ParsedHook>> {
    let mut hooks = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| anyhow!("hooks[{}] must be an object", index))?;
        if let Some(hook) = parse_flat_hook(object, None, None, None)
            .with_context(|| format!("invalid hooks[{}]", index))?
        {
            hooks.push(hook);
        }
    }
    Ok(hooks)
}

fn parse_event_map(events: &Map<String, Value>) -> Result<Vec<ParsedHook>> {
    let mut hooks = Vec::new();
    for (event_name, entries) in events {
        let Some(hook_type) = parse_hook_type(event_name) else {
            // Codex and Claude support lifecycle events that Mitsuro's tool-hook
            // executor does not. Ignoring them is safe; executing them under a
            // different event would not be.
            continue;
        };
        let entries = entries
            .as_array()
            .ok_or_else(|| anyhow!("hooks.{} must be an array", event_name))?;
        for (entry_index, entry) in entries.iter().enumerate() {
            let object = entry.as_object().ok_or_else(|| {
                anyhow!("hooks.{}[{}] must be an object", event_name, entry_index)
            })?;
            let inherited_matcher = optional_string(object, &["matcher", "tool_pattern"])?;
            let inherited_timeout = optional_timeout(object)?;

            if let Some(nested) = object.get("hooks") {
                let nested = nested.as_array().ok_or_else(|| {
                    anyhow!(
                        "hooks.{}[{}].hooks must be an array",
                        event_name,
                        entry_index
                    )
                })?;
                for (nested_index, command_hook) in nested.iter().enumerate() {
                    let command_hook = command_hook.as_object().ok_or_else(|| {
                        anyhow!(
                            "hooks.{}[{}].hooks[{}] must be an object",
                            event_name,
                            entry_index,
                            nested_index
                        )
                    })?;
                    if let Some(hook) = parse_flat_hook(
                        command_hook,
                        Some(hook_type),
                        inherited_matcher.as_deref(),
                        inherited_timeout,
                    )
                    .with_context(|| {
                        format!(
                            "invalid hooks.{}[{}].hooks[{}]",
                            event_name, entry_index, nested_index
                        )
                    })? {
                        hooks.push(hook);
                    }
                }
            } else if let Some(hook) = parse_flat_hook(
                object,
                Some(hook_type),
                inherited_matcher.as_deref(),
                inherited_timeout,
            )
            .with_context(|| format!("invalid hooks.{}[{}]", event_name, entry_index))?
            {
                hooks.push(hook);
            }
        }
    }
    Ok(hooks)
}

fn parse_flat_hook(
    object: &Map<String, Value>,
    inherited_type: Option<UserHookType>,
    inherited_matcher: Option<&str>,
    inherited_timeout: Option<u64>,
) -> Result<Option<ParsedHook>> {
    if let Some(kind) = optional_string(object, &["type", "kind"])? {
        if !kind.eq_ignore_ascii_case("command") {
            return Ok(None);
        }
    }

    let hook_type = match inherited_type {
        Some(hook_type) => hook_type,
        None => {
            let event = optional_string(object, &["hook_type", "event"])?
                .ok_or_else(|| anyhow!("command hook is missing hook_type/event"))?;
            parse_hook_type(&event)
                .ok_or_else(|| anyhow!("unsupported command hook event '{}'", event))?
        }
    };
    let command = required_string(object, "command")?;
    if command.len() > MAX_COMMAND_BYTES {
        bail!("command exceeds {} bytes", MAX_COMMAND_BYTES);
    }

    let pattern = optional_string(object, &["tool_pattern", "matcher"])?
        .or_else(|| inherited_matcher.map(ToOwned::to_owned))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| ".*".to_string());
    if pattern.len() > MAX_PATTERN_BYTES {
        bail!("tool matcher exceeds {} bytes", MAX_PATTERN_BYTES);
    }
    Regex::new(&pattern).with_context(|| format!("invalid tool matcher regex '{}'", pattern))?;

    let timeout_seconds = optional_timeout(object)?
        .or(inherited_timeout)
        .unwrap_or(30);
    if !(1..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        bail!(
            "timeout must be between 1 and {} seconds",
            MAX_TIMEOUT_SECONDS
        );
    }

    let enabled = match object.get("enabled") {
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => bail!("enabled must be a boolean"),
        None => true,
    };

    Ok(Some(ParsedHook {
        hook_type,
        tool_pattern: pattern,
        command,
        enabled,
        timeout_seconds,
    }))
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String> {
    let value = object
        .get(key)
        .ok_or_else(|| anyhow!("command hook is missing '{}'", key))?;
    let value = value
        .as_str()
        .ok_or_else(|| anyhow!("'{}' must be a string", key))?
        .trim();
    if value.is_empty() {
        bail!("'{}' cannot be empty", key);
    }
    Ok(value.to_string())
}

fn optional_string(object: &Map<String, Value>, keys: &[&str]) -> Result<Option<String>> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            return value
                .as_str()
                .map(|value| Some(value.to_string()))
                .ok_or_else(|| anyhow!("'{}' must be a string", key));
        }
    }
    Ok(None)
}

fn optional_timeout(object: &Map<String, Value>) -> Result<Option<u64>> {
    for key in ["timeout_seconds", "timeout"] {
        if let Some(value) = object.get(key) {
            return value
                .as_u64()
                .map(Some)
                .ok_or_else(|| anyhow!("'{}' must be a positive integer", key));
        }
    }
    Ok(None)
}

fn looks_like_flat_hook(object: &Map<String, Value>) -> bool {
    object.contains_key("command")
        && (object.contains_key("hook_type") || object.contains_key("event"))
}

fn parse_hook_type(name: &str) -> Option<UserHookType> {
    let normalized = name
        .chars()
        .filter(|character| !matches!(character, '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    match normalized.as_str() {
        "pretooluse" | "beforetooluse" => Some(UserHookType::PreToolUse),
        "posttooluse" | "aftertooluse" => Some(UserHookType::PostToolUse),
        "notification" => Some(UserHookType::Notification),
        "userpromptsubmit" | "beforeprompt" => Some(UserHookType::UserPromptSubmit),
        _ => None,
    }
}

fn stable_hook_id(plugin_id: &str, config_path: &Path, index: usize, hook: &ParsedHook) -> String {
    let mut digest = Sha256::new();
    digest.update(plugin_id.as_bytes());
    digest.update([0]);
    digest.update(config_path.as_os_str().to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(index.to_le_bytes());
    digest.update([hook.hook_type as u8]);
    digest.update(hook.tool_pattern.as_bytes());
    digest.update([0]);
    digest.update(hook.command.as_bytes());
    let digest = digest.finalize();
    let suffix = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("package:{plugin_id}:{suffix}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_document_hooks;
    use crate::agent::user_hooks::UserHookType;

    #[test]
    fn parses_mitsuro_flat_hooks() {
        let hooks = parse_document_hooks(&json!({
            "hooks": [
                {
                    "hook_type": "PreToolUse",
                    "tool_pattern": "Write|Edit",
                    "command": "./check.sh"
                },
                {
                    "event": "post_tool_use",
                    "matcher": "Bash",
                    "command": "./audit.sh",
                    "timeout_seconds": 12
                }
            ]
        }))
        .expect("flat hook config should parse");

        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].hook_type, UserHookType::PreToolUse);
        assert_eq!(hooks[1].hook_type, UserHookType::PostToolUse);
        assert_eq!(hooks[1].timeout_seconds, 12);
    }

    #[test]
    fn parses_claude_nested_command_hooks_and_ignores_other_kinds() {
        let hooks = parse_document_hooks(&json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash|Write",
                        "hooks": [
                            {"type": "command", "command": "./guard.sh"},
                            {"type": "prompt", "prompt": "Review this"}
                        ]
                    }
                ],
                "SessionStart": [
                    {"type": "command", "command": "./start.sh"}
                ]
            }
        }))
        .expect("Claude hook config should parse");

        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].hook_type, UserHookType::PreToolUse);
        assert_eq!(hooks[0].tool_pattern, "Bash|Write");
        assert_eq!(hooks[0].command, "./guard.sh");
    }

    #[test]
    fn rejects_invalid_matcher() {
        let error = parse_document_hooks(&json!([{
            "hook_type": "PreToolUse",
            "tool_pattern": "[",
            "command": "true"
        }]))
        .expect_err("invalid matcher must fail closed");

        assert!(format!("{error:#}").contains("invalid tool matcher"));
    }
}
