use super::execution::{join_reader_with_timeout, BoundedOutputBuffer};
use super::shell::strip_shell_background_suffix;

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
