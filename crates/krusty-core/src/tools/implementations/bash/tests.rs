use super::execution::{join_reader_with_timeout, BoundedOutputBuffer};
use super::shell::strip_shell_background_suffix;
use super::{output_spool_path, BashTool};
use crate::tools::registry::Tool;
use crate::tools::ToolContext;
use serde_json::json;

#[test]
fn strip_shell_background_suffix_accepts_simple_suffix() {
    let parsed = strip_shell_background_suffix("npm run dev &");
    assert_eq!(parsed.as_deref(), Some("npm run dev"));
}

#[test]
fn strip_shell_background_suffix_rejects_quoted_ampersand() {
    let parsed = strip_shell_background_suffix("echo '&'");
    assert!(parsed.is_none());
}

#[test]
fn strip_shell_background_suffix_rejects_escaped_ampersand() {
    let parsed = strip_shell_background_suffix(r"echo foo \&");
    assert!(parsed.is_none());
}

#[test]
fn strip_shell_background_suffix_rejects_double_ampersand() {
    let parsed = strip_shell_background_suffix("echo hi &&");
    assert!(parsed.is_none());
}

#[test]
fn bounded_output_buffer_keeps_recent_lines() {
    let mut buffer = BoundedOutputBuffer::new(3, 1024);
    buffer.push_line("l1");
    buffer.push_line("l2");
    buffer.push_line("l3");
    buffer.push_line("l4");

    let text = buffer.into_text();
    assert!(!text.contains("l1"));
    assert!(text.contains("l2"));
    assert!(text.contains("l3"));
    assert!(text.contains("l4"));
}

#[test]
fn bounded_output_buffer_clips_to_max_bytes() {
    let mut buffer = BoundedOutputBuffer::new(100, 10);
    buffer.push_line("12345");
    buffer.push_line("67890");
    buffer.push_line("abcdef");

    let text = buffer.into_text();
    assert!(text.len() <= 200);
    assert!(text.contains("abcdef") || text.contains("bcdef"));
}

#[tokio::test]
async fn join_reader_with_timeout_does_not_double_poll_completed_handle() {
    let handle = tokio::spawn(async {});
    join_reader_with_timeout(handle).await;
}

#[test]
fn output_spool_is_workspace_local_and_session_scoped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: Some("session/with spaces".to_string()),
        ..Default::default()
    };

    let path = output_spool_path(&ctx);
    assert!(path.starts_with(temp.path().join(".krusty/tool-output")));
    assert!(path.to_string_lossy().contains("session_with_spaces"));
    assert_eq!(
        path.extension().and_then(|value| value.to_str()),
        Some("log")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn truncated_output_keeps_recoverable_full_log() {
    let temp = tempfile::tempdir().expect("tempdir");
    let working_dir = temp.path().canonicalize().expect("canonical tempdir");
    let ctx = ToolContext {
        working_dir: working_dir.clone(),
        session_id: Some("test-session".to_string()),
        sandbox_root: Some(working_dir.clone()),
        ..Default::default()
    };
    let command = "i=1; while [ $i -le 5000 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done";

    let result = BashTool.execute(json!({"command": command}), &ctx).await;
    assert!(!result.is_error, "{}", result.output);

    let envelope: serde_json::Value = serde_json::from_str(&result.output).expect("tool JSON");
    let preview = envelope["data"]["output"].as_str().expect("output preview");
    assert!(preview.contains("Output truncated"));
    assert!(preview.contains("Full output saved to"));
    assert!(!preview.contains("line-1\n"));
    assert!(preview.contains("line-5000"));

    let directory = working_dir.join(".krusty/tool-output/test-session");
    let paths = std::fs::read_dir(directory)
        .expect("output directory")
        .map(|entry| entry.expect("output entry").path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    let full = std::fs::read_to_string(&paths[0]).expect("full output log");
    assert!(full.contains("line-1\n"));
    assert!(full.contains("line-5000\n"));
}
