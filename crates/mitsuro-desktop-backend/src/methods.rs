//! Canonical registry of Codex app-server **client** methods.
//!
//! Source inventory: generated from the Codex CLI named in
//! `fixtures/codex-protocol-version.txt`. `fixtures/client-methods.txt` is the
//! complete stable + experimental surface, while `stable-client-methods.txt`
//! records the methods available without `experimentalApi` negotiation.
//! Inventory is not implementation proof.
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
    "plugin/search",
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
    "thread/section/move",
    "thread/settings/update",
    "thread/shellCommand",
    "thread/start",
    "thread/turns/list",
    "thread/unarchive",
    "thread/unsubscribe",
    "threadSection/create",
    "threadSection/delete",
    "threadSection/list",
    "threadSection/update",
    "turn/interrupt",
    "turn/start",
    "turn/steer",
    "windowsSandbox/readiness",
    "windowsSandbox/setupStart",
];

/// Number of registered client methods (convenience for tests / UI).
pub const CLIENT_METHOD_COUNT: usize = CLIENT_METHODS.len();

/// Codex methods currently represented by a typed backend adapter.
///
/// Everything else in [`CLIENT_METHODS`] is still reachable through Codex's raw
/// JSON-RPC transport, but is not yet a complete product capability. Keeping this
/// list explicit prevents protocol reachability from being mistaken for UI parity.
pub const TYPED_CLIENT_METHODS: &[&str] = &[
    "account/login/cancel",
    "account/login/start",
    "account/logout",
    "account/rateLimits/read",
    "account/read",
    "account/usage/read",
    "collaborationMode/list",
    "config/read",
    "environment/add",
    "environment/info",
    "environment/status",
    "fs/getMetadata",
    "fs/readDirectory",
    "fs/readFile",
    "fuzzyFileSearch",
    "fuzzyFileSearch/sessionStart",
    "fuzzyFileSearch/sessionStop",
    "fuzzyFileSearch/sessionUpdate",
    "initialize",
    "mcpServer/tool/call",
    "mcpServerStatus/list",
    "model/list",
    "plugin/installed",
    "plugin/list",
    "plugin/read",
    "process/kill",
    "process/resizePty",
    "process/spawn",
    "process/writeStdin",
    "review/start",
    "skills/list",
    "thread/archive",
    "thread/compact/start",
    "thread/delete",
    "thread/fork",
    "thread/goal/clear",
    "thread/goal/get",
    "thread/goal/set",
    "thread/list",
    "thread/name/set",
    "thread/read",
    "thread/resume",
    "thread/search",
    "thread/start",
    "thread/unarchive",
    "turn/interrupt",
    "turn/start",
    "turn/steer",
];

pub const TYPED_CLIENT_METHOD_COUNT: usize = TYPED_CLIENT_METHODS.len();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMethodCoverage {
    /// A typed adapter is available to product code.
    Typed,
    /// The Codex transport can invoke it, but no typed product adapter exists yet.
    RawOnly,
    /// Not present in the generated protocol inventory.
    Unknown,
}

pub fn client_method_coverage(method: &str) -> ClientMethodCoverage {
    if TYPED_CLIENT_METHODS.contains(&method) {
        ClientMethodCoverage::Typed
    } else if is_known_client_method(method) {
        ClientMethodCoverage::RawOnly
    } else {
        ClientMethodCoverage::Unknown
    }
}

/// Current methods available without `initialize.capabilities.experimentalApi`.
pub const STABLE_CLIENT_METHODS_TEXT: &str = include_str!("../fixtures/stable-client-methods.txt");

/// Number of methods in the generated stable app-server contract.
pub const STABLE_CLIENT_METHOD_COUNT: usize = 95;

/// Number of additional methods exposed after experimental API negotiation.
pub const EXPERIMENTAL_ONLY_CLIENT_METHOD_COUNT: usize =
    CLIENT_METHOD_COUNT - STABLE_CLIENT_METHOD_COUNT;

/// Returns `true` if `m` is in [`CLIENT_METHODS`].
pub fn is_known_client_method(m: &str) -> bool {
    CLIENT_METHODS.binary_search(&m).is_ok() || CLIENT_METHODS.contains(&m)
}

/// Returns true when `method` is part of the generated stable contract.
pub fn is_stable_client_method(method: &str) -> bool {
    STABLE_CLIENT_METHODS_TEXT
        .lines()
        .any(|candidate| candidate == method)
}

/// Returns true when a known method requires experimental API negotiation.
pub fn requires_experimental_api(method: &str) -> bool {
    is_known_client_method(method) && !is_stable_client_method(method)
}

/// Path to the maintained protocol method inventory.
pub fn client_methods_txt_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/client-methods.txt")
}

pub fn stable_client_methods_txt_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/stable-client-methods.txt")
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

pub fn load_stable_client_methods_from_bar() -> std::io::Result<Vec<String>> {
    let text = std::fs::read_to_string(stable_client_methods_txt_path())?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
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
        assert_eq!(
            CLIENT_METHOD_COUNT, 133,
            "Codex 0.147.0 experimental schema documents 133 client methods"
        );
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

    #[test]
    fn every_method_has_honest_typed_or_raw_only_coverage() {
        assert_eq!(TYPED_CLIENT_METHOD_COUNT, 48);
        for method in TYPED_CLIENT_METHODS {
            assert!(
                is_known_client_method(method),
                "typed method is unknown: {method}"
            );
            assert_eq!(client_method_coverage(method), ClientMethodCoverage::Typed);
        }
        let raw_only = CLIENT_METHODS
            .iter()
            .filter(|method| client_method_coverage(method) == ClientMethodCoverage::RawOnly)
            .count();
        assert_eq!(raw_only, CLIENT_METHOD_COUNT - TYPED_CLIENT_METHOD_COUNT);
        assert_eq!(
            client_method_coverage("future/not-yet-generated"),
            ClientMethodCoverage::Unknown
        );
    }

    #[test]
    fn stable_and_experimental_methods_are_classified_from_generated_inventory() {
        let stable =
            load_stable_client_methods_from_bar().expect("stable-client-methods.txt readable");
        assert_eq!(stable.len(), STABLE_CLIENT_METHOD_COUNT);
        assert_eq!(EXPERIMENTAL_ONLY_CLIENT_METHOD_COUNT, 38);
        assert!(is_stable_client_method("thread/section/move"));
        assert!(!requires_experimental_api("thread/section/move"));
        assert!(requires_experimental_api("process/spawn"));
        assert!(requires_experimental_api("thread/realtime/start"));
        assert!(!requires_experimental_api("not/a/real/method"));
        for method in stable {
            assert!(
                is_known_client_method(&method),
                "stable method missing: {method}"
            );
            assert!(is_stable_client_method(&method));
        }
    }
}
