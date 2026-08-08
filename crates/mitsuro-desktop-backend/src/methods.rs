//! Canonical registry of Codex app-server **client** methods.
//!
//! Source inventory: `fixtures/client-methods.txt` (keep in sync with the
//! supported Codex app-server protocol; inventory is not implementation proof).
//! Universal RPC entry point: [`crate::backend::AgentBackend::call_raw`].

/// All known client JSON-RPC methods from the protocol bar.
///
/// Length must match the maintained protocol inventory.
pub const CLIENT_METHODS: &[&str] = &[
    "account/login/cancel",
    "account/login/start",
    "account/logout",
    "account/rateLimitResetCredit/consume",
    "account/rateLimits/read",
    "account/read",
    "account/sendAddCreditsNudgeEmail",
    "account/usage/read",
    "account/workspaceMessages/read",
    "app/installed",
    "app/list",
    "app/read",
    "collaborationMode/list",
    "command/exec",
    "command/exec/resize",
    "command/exec/terminate",
    "command/exec/write",
    "config/batchWrite",
    "config/mcpServer/reload",
    "config/read",
    "config/value/write",
    "configRequirements/read",
    "environment/add",
    "environment/info",
    "environment/status",
    "experimentalFeature/enablement/set",
    "experimentalFeature/list",
    "externalAgentConfig/detect",
    "externalAgentConfig/import",
    "externalAgentConfig/import/readHistories",
    "externalAgentConfig/import/recordHistory",
    "feedback/upload",
    "fs/copy",
    "fs/createDirectory",
    "fs/getMetadata",
    "fs/readDirectory",
    "fs/readFile",
    "fs/remove",
    "fs/unwatch",
    "fs/watch",
    "fs/writeFile",
    "fuzzyFileSearch",
    "fuzzyFileSearch/sessionStart",
    "fuzzyFileSearch/sessionStop",
    "fuzzyFileSearch/sessionUpdate",
    "hooks/list",
    "initialize",
    "marketplace/add",
    "marketplace/remove",
    "marketplace/upgrade",
    "mcpServer/oauth/login",
    "mcpServer/resource/read",
    "mcpServer/tool/call",
    "mcpServerStatus/list",
    "memory/reset",
    "mock/experimentalMethod",
    "model/list",
    "modelProvider/capabilities/read",
    "permissionProfile/list",
    "plugin/install",
    "plugin/installed",
    "plugin/list",
    "plugin/read",
    "plugin/share/checkout",
    "plugin/share/delete",
    "plugin/share/list",
    "plugin/share/save",
    "plugin/share/updateTargets",
    "plugin/skill/read",
    "plugin/uninstall",
    "process/kill",
    "process/resizePty",
    "process/spawn",
    "process/writeStdin",
    "remoteControl/client/list",
    "remoteControl/client/revoke",
    "remoteControl/disable",
    "remoteControl/enable",
    "remoteControl/pairing/start",
    "remoteControl/pairing/status",
    "remoteControl/status/read",
    "review/start",
    "skills/config/write",
    "skills/extraRoots/set",
    "skills/list",
    "thread/approveGuardianDeniedAction",
    "thread/archive",
    "thread/backgroundTerminals/clean",
    "thread/backgroundTerminals/list",
    "thread/backgroundTerminals/terminate",
    "thread/compact/start",
    "thread/decrement_elicitation",
    "thread/delete",
    "thread/fork",
    "thread/goal/clear",
    "thread/goal/get",
    "thread/goal/set",
    "thread/increment_elicitation",
    "thread/inject_items",
    "thread/items/list",
    "thread/list",
    "thread/loaded/list",
    "thread/memoryMode/set",
    "thread/metadata/update",
    "thread/name/set",
    "thread/read",
    "thread/realtime/appendAudio",
    "thread/realtime/appendSpeech",
    "thread/realtime/appendText",
    "thread/realtime/listVoices",
    "thread/realtime/start",
    "thread/realtime/stop",
    "thread/resume",
    "thread/rollback",
    "thread/search",
    "thread/searchOccurrences",
    "thread/settings/update",
    "thread/shellCommand",
    "thread/start",
    "thread/turns/list",
    "thread/unarchive",
    "thread/unsubscribe",
    "turn/interrupt",
    "turn/start",
    "turn/steer",
    "windowsSandbox/readiness",
    "windowsSandbox/setupStart",
];

/// Number of registered client methods (convenience for tests / UI).
pub const CLIENT_METHOD_COUNT: usize = CLIENT_METHODS.len();

/// Returns `true` if `m` is in [`CLIENT_METHODS`].
pub fn is_known_client_method(m: &str) -> bool {
    CLIENT_METHODS.binary_search(&m).is_ok() || CLIENT_METHODS.contains(&m)
}

/// Path to the maintained protocol method inventory.
pub fn client_methods_txt_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/client-methods.txt")
}

/// Load method names from `client-methods.txt` on disk (for parity tests).
pub fn load_client_methods_from_bar() -> std::io::Result<Vec<String>> {
    let path = client_methods_txt_path();
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_client_methods_are_known() {
        let disk = load_client_methods_from_bar().expect("client-methods.txt readable");
        assert_eq!(
            CLIENT_METHODS.len(),
            disk.len(),
            "CLIENT_METHODS len {} != client-methods.txt line count {}",
            CLIENT_METHODS.len(),
            disk.len()
        );
        assert_eq!(CLIENT_METHOD_COUNT, 127, "bar documents 127 client methods");
        for (i, (reg, file)) in CLIENT_METHODS.iter().zip(disk.iter()).enumerate() {
            assert_eq!(
                *reg,
                file.as_str(),
                "mismatch at index {i}: registry={reg:?} file={file:?}"
            );
            assert!(
                is_known_client_method(reg),
                "registry entry not known: {reg}"
            );
        }
        assert!(!is_known_client_method("not/a/real/method"));
        assert!(is_known_client_method("initialize"));
        assert!(is_known_client_method("thread/list"));
        assert!(is_known_client_method("process/spawn"));
    }
}
