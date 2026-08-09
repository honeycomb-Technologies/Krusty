//! `process/*` app-server methods and notification helpers.
//!
//! Typed Codex app-server process request and notification shapes.
//!
//! - Client methods: `process/spawn`, `process/writeStdin`, `process/resizePty`, `process/kill`
//! - Notifications: `process/outputDelta`, `process/exited`

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// PTY size in character cells for `process/spawn` / `process/resizePty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl ProcessTerminalSize {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }
}

/// Stream label for `process/outputDelta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessOutputStream {
    Stdout,
    Stderr,
}

impl ProcessOutputStream {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "stderr" => Self::Stderr,
            _ => Self::Stdout,
        }
    }
}

/// Params for `process/spawn`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSpawnParams {
    /// Argv vector (must be non-empty).
    pub command: Vec<String>,
    /// Client-supplied, connection-scoped process handle.
    pub process_handle: String,
    /// Absolute working directory.
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_stdin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_stdout_stderr: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes_cap: Option<Option<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<ProcessTerminalSize>,
}

impl ProcessSpawnParams {
    /// Spawn with streaming stdout/stderr + stdin (typical terminal panel defaults).
    pub fn streaming(
        command: Vec<String>,
        process_handle: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Self {
        Self {
            command,
            process_handle: process_handle.into(),
            cwd: cwd.into(),
            tty: Some(false),
            stream_stdin: Some(true),
            stream_stdout_stderr: Some(true),
            output_bytes_cap: None,
            timeout_ms: None,
            env: None,
            size: None,
        }
    }

    /// Shell command via `bash -lc`.
    pub fn bash_lc(
        script: impl Into<String>,
        process_handle: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Self {
        Self::streaming(
            vec!["bash".into(), "-lc".into(), script.into()],
            process_handle,
            cwd,
        )
    }
}

/// Successful response for `process/spawn` (schema is empty; fixture may echo handle).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSpawnResponse {
    /// Not in the official empty schema — fixture / UI convenience only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_handle: Option<String>,
}

/// Params for `process/writeStdin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessWriteStdinParams {
    pub process_handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_stdin: Option<bool>,
}

impl ProcessWriteStdinParams {
    pub fn text(process_handle: impl Into<String>, text: &str) -> Self {
        Self {
            process_handle: process_handle.into(),
            delta_base64: Some(encode_base64(text.as_bytes())),
            close_stdin: None,
        }
    }

    pub fn close(process_handle: impl Into<String>) -> Self {
        Self {
            process_handle: process_handle.into(),
            delta_base64: None,
            close_stdin: Some(true),
        }
    }
}

/// Empty success for `process/writeStdin`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessWriteStdinResponse {}

/// Params for `process/kill`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessKillParams {
    pub process_handle: String,
}

impl ProcessKillParams {
    pub fn new(process_handle: impl Into<String>) -> Self {
        Self {
            process_handle: process_handle.into(),
        }
    }
}

/// Empty success for `process/kill`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessKillResponse {}

/// Params for `process/resizePty`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResizePtyParams {
    pub process_handle: String,
    pub size: ProcessTerminalSize,
}

impl ProcessResizePtyParams {
    pub fn new(process_handle: impl Into<String>, rows: u16, cols: u16) -> Self {
        Self {
            process_handle: process_handle.into(),
            size: ProcessTerminalSize::new(rows, cols),
        }
    }
}

/// Empty success for `process/resizePty`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessResizePtyResponse {}

// ---------------------------------------------------------------------------
// Base64 (minimal, no extra dep — RFC 4648 standard alphabet)
// ---------------------------------------------------------------------------

const B64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as standard base64 (no padding stripped).
pub fn encode_base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Decode standard base64 (ignores whitespace; tolerates missing padding).
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    let mut buf = clean;
    while !buf.len().is_multiple_of(4) {
        buf.push(b'=');
    }
    let mut out = Vec::with_capacity(buf.len() / 4 * 3);
    for chunk in buf.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    pad += 1;
                    0
                }
                other => {
                    return Err(format!("invalid base64 byte: {other}"));
                }
            };
            n = (n << 6) | u32::from(v);
            let _ = i;
        }
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}

/// Decode base64 to lossy UTF-8 string (UI convenience).
pub fn decode_base64_lossy(input: &str) -> String {
    match decode_base64(input) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Notification → event field extractors
// ---------------------------------------------------------------------------

/// Parse `process/outputDelta` params into structured fields.
pub fn parse_process_output_delta(
    params: Option<&Value>,
) -> (String, ProcessOutputStream, String, String, bool) {
    let p = params.cloned().unwrap_or(Value::Null);
    let handle = p
        .get("processHandle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stream = p
        .get("stream")
        .and_then(|v| v.as_str())
        .map(ProcessOutputStream::from_str_lossy)
        .unwrap_or(ProcessOutputStream::Stdout);
    let delta_base64 = p
        .get("deltaBase64")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let delta = decode_base64_lossy(&delta_base64);
    let cap_reached = p
        .get("capReached")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    (handle, stream, delta_base64, delta, cap_reached)
}

/// Parse `process/exited` params into structured fields.
pub fn parse_process_exited(params: Option<&Value>) -> (String, i32, String, bool, String, bool) {
    let p = params.cloned().unwrap_or(Value::Null);
    let handle = p
        .get("processHandle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let exit_code = p.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let stdout = p
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stdout_cap = p
        .get("stdoutCapReached")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let stderr = p
        .get("stderr")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stderr_cap = p
        .get("stderrCapReached")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    (handle, exit_code, stdout, stdout_cap, stderr, stderr_cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        let samples: &[&[u8]] = &[b"", b"f", b"fo", b"foo", b"hello mitsuro", b"\0\x01\x02"];
        for s in samples {
            let enc = encode_base64(s);
            let dec = decode_base64(&enc).unwrap();
            assert_eq!(&dec, s, "enc={enc}");
        }
    }

    #[test]
    fn process_spawn_params_serialize_camel_case() {
        let p = ProcessSpawnParams::bash_lc("echo hi", "ph-1", "/tmp");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["processHandle"], "ph-1");
        assert_eq!(v["streamStdin"], true);
        assert_eq!(v["streamStdoutStderr"], true);
        assert!(v["command"].as_array().unwrap().len() == 3);
    }
}
