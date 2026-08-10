//! Typed Codex `command/exec*` contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::process::encode_base64;
use crate::protocol::SandboxPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecTerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl CommandExecTerminalSize {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }
}

/// Run one standalone argv vector in the app-server sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecParams {
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_stdin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_stdout_stderr: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes_cap: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_output_cap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_timeout: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<CommandExecTerminalSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<SandboxPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
}

impl CommandExecParams {
    /// Buffered standalone command for bounded non-interactive operations and tests.
    pub fn buffered(command: Vec<String>, cwd: impl Into<String>, timeout_ms: u64) -> Self {
        Self {
            command,
            process_id: None,
            tty: None,
            stream_stdin: None,
            stream_stdout_stderr: None,
            output_bytes_cap: Some(128 * 1024),
            disable_output_cap: None,
            disable_timeout: None,
            timeout_ms: Some(timeout_ms),
            cwd: Some(cwd.into()),
            env: None,
            size: None,
            sandbox_policy: None,
            permission_profile: None,
        }
    }

    /// Interactive shell request used by the native Terminal surface.
    pub fn terminal_shell(
        script: impl Into<String>,
        process_id: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Self {
        Self {
            command: vec!["bash".to_owned(), "-lc".to_owned(), script.into()],
            process_id: Some(process_id.into()),
            tty: Some(true),
            stream_stdin: Some(true),
            stream_stdout_stderr: Some(true),
            output_bytes_cap: Some(128 * 1024),
            disable_output_cap: None,
            disable_timeout: Some(true),
            timeout_ms: None,
            cwd: Some(cwd.into()),
            env: None,
            size: Some(CommandExecTerminalSize::new(30, 120)),
            sandbox_policy: None,
            permission_profile: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecWriteParams {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_stdin: Option<bool>,
}

impl CommandExecWriteParams {
    pub fn text(process_id: impl Into<String>, text: &str) -> Self {
        Self {
            process_id: process_id.into(),
            delta_base64: Some(encode_base64(text.as_bytes())),
            close_stdin: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecWriteResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecTerminateParams {
    pub process_id: String,
}

impl CommandExecTerminateParams {
    pub fn new(process_id: impl Into<String>) -> Self {
        Self {
            process_id: process_id.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecTerminateResponse {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecResizeParams {
    pub process_id: String,
    pub size: CommandExecTerminalSize,
}

impl CommandExecResizeParams {
    pub fn new(process_id: impl Into<String>, rows: u16, cols: u16) -> Self {
        Self {
            process_id: process_id.into(),
            size: CommandExecTerminalSize::new(rows, cols),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecResizeResponse {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandExecOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecOutputDeltaNotification {
    pub process_id: String,
    pub stream: CommandExecOutputStream,
    pub delta_base64: String,
    pub cap_reached: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_request_matches_generated_contract() {
        let params = CommandExecParams::terminal_shell("printf hi", "term-1", "/tmp");
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(
            value["command"],
            serde_json::json!(["bash", "-lc", "printf hi"])
        );
        assert_eq!(value["processId"], "term-1");
        assert_eq!(value["tty"], true);
        assert_eq!(value["streamStdin"], true);
        assert_eq!(value["streamStdoutStderr"], true);
        assert_eq!(value["outputBytesCap"], 131_072);
        assert_eq!(value["disableTimeout"], true);
        assert_eq!(value["cwd"], "/tmp");
        assert_eq!(value["size"], serde_json::json!({"rows": 30, "cols": 120}));
        assert!(value.get("sandboxPolicy").is_none());
        assert!(value.get("permissionProfile").is_none());
    }

    #[test]
    fn buffered_request_is_bounded_and_noninteractive() {
        let params = CommandExecParams::buffered(
            vec!["printf".to_owned(), "hello".to_owned()],
            "/tmp",
            5_000,
        );
        let value = serde_json::to_value(params).unwrap();
        assert_eq!(value["command"], serde_json::json!(["printf", "hello"]));
        assert_eq!(value["outputBytesCap"], 131_072);
        assert_eq!(value["timeoutMs"], 5_000);
        assert_eq!(value["cwd"], "/tmp");
        assert!(value.get("processId").is_none());
        assert!(value.get("tty").is_none());
        assert!(value.get("streamStdin").is_none());
        assert!(value.get("disableTimeout").is_none());
    }

    #[test]
    fn write_and_output_delta_are_base64_exact() {
        let write =
            serde_json::to_value(CommandExecWriteParams::text("term-1", "hello\n")).unwrap();
        assert_eq!(
            write,
            serde_json::json!({"processId": "term-1", "deltaBase64": "aGVsbG8K"})
        );
        let output: CommandExecOutputDeltaNotification =
            serde_json::from_value(serde_json::json!({
                "processId": "term-1",
                "stream": "stderr",
                "deltaBase64": "b29wcw==",
                "capReached": true
            }))
            .unwrap();
        assert_eq!(output.stream, CommandExecOutputStream::Stderr);
        assert!(output.cap_reached);
    }
}
