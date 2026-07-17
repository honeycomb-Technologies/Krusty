#[cfg(unix)]
use super::execution::execute_foreground;
use super::execution::{join_reader_with_timeout, BoundedOutputBuffer};
use super::shell::strip_shell_background_suffix;
#[cfg(unix)]
use super::shell::{build_shell_command, configure_foreground_process_group};
use super::{output_spool_path, BashTool};
use crate::process::ProcessRegistry;
use crate::tools::registry::Tool;
use crate::tools::ToolContext;
use serde_json::json;
use std::sync::Arc;

#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
struct TestProcessCleanup {
    pids: Vec<u32>,
    armed: bool,
}

#[cfg(unix)]
impl TestProcessCleanup {
    fn new(pids: Vec<u32>) -> Self {
        Self { pids, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for TestProcessCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        for pid in &self.pids {
            if let Ok(pid) = libc::pid_t::try_from(*pid) {
                // Test-only best effort: every process is uniquely created by
                // this test and is killed individually if an assertion fails.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
async fn wait_for_processes_to_exit(pids: &[u32]) -> bool {
    for _ in 0..200 {
        if pids.iter().all(|pid| !process_is_alive(*pid)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[cfg(unix)]
async fn wait_for_pid_file(path: &std::path::Path) -> (u32, u32) {
    for _ in 0..200 {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let pids = raw
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
                .collect::<Vec<_>>();
            if pids.len() == 2 {
                return (pids[0], pids[1]);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process tree did not write PID file {}", path.display());
}

#[cfg(unix)]
fn foreground_process_tree_command(
    working_dir: &std::path::Path,
    pid_file_name: &str,
) -> tokio::process::Command {
    let script_name = "foreground-process-tree.sh";
    std::fs::write(
        working_dir.join(script_name),
        "#!/bin/sh\nsleep 60 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$1\"\nwait \"$child\"\n",
    )
    .expect("write process-tree script");

    let ctx = ToolContext {
        working_dir: working_dir.to_path_buf(),
        ..Default::default()
    };
    let mut command = build_shell_command(&format!("sh {script_name} {pid_file_name}"), &ctx);
    configure_foreground_process_group(&mut command);
    command.kill_on_drop(true);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

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

#[cfg(unix)]
#[tokio::test]
async fn background_start_reports_fast_failure_with_process_handle_and_stderr() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(ProcessRegistry::new());
    let ctx = ToolContext::with_process_registry(
        temp.path().canonicalize().expect("canonical tempdir"),
        registry,
    );

    let result = BashTool
        .execute(
            json!({
                "command": "echo 'address already in use' >&2; exit 7",
                "run_in_background": true
            }),
            &ctx,
        )
        .await;

    assert!(result.is_error, "{}", result.output);
    let envelope: serde_json::Value = serde_json::from_str(&result.output).expect("tool JSON");
    assert_eq!(
        envelope["error"]["code"].as_str(),
        Some("background_start_failed")
    );
    assert_eq!(envelope["data"]["status"].as_str(), Some("failed"));
    assert!(envelope["data"]["process_id"].is_string());
    assert!(envelope["data"]["error"]
        .as_str()
        .is_some_and(|error| error.contains("address already in use")));
}

#[cfg(unix)]
#[tokio::test]
async fn background_start_returns_trackable_running_process() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(ProcessRegistry::new());
    let ctx = ToolContext::with_process_registry(
        temp.path().canonicalize().expect("canonical tempdir"),
        Arc::clone(&registry),
    );

    let result = BashTool
        .execute(
            json!({"command": "sleep 5", "run_in_background": true}),
            &ctx,
        )
        .await;

    assert!(!result.is_error, "{}", result.output);
    let envelope: serde_json::Value = serde_json::from_str(&result.output).expect("tool JSON");
    let process_id = envelope["data"]["process_id"].as_str().expect("process id");
    assert_eq!(envelope["data"]["status"].as_str(), Some("running"));
    assert!(envelope["data"]["next_action"]
        .as_str()
        .is_some_and(|guidance| guidance.contains("processes")));

    registry.kill(process_id).await.expect("kill test process");
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_entire_foreground_process_group() {
    let temp = tempfile::tempdir().expect("tempdir");
    let working_dir = temp.path().canonicalize().expect("canonical tempdir");
    let pid_file_name = "timeout-process-tree.pids";
    let pid_file = working_dir.join(pid_file_name);
    let command = foreground_process_tree_command(&working_dir, pid_file_name);

    let result = execute_foreground(
        command,
        Duration::from_millis(750),
        None,
        working_dir.join("timeout-output.log"),
    )
    .await;
    let (leader_pid, descendant_pid) = wait_for_pid_file(&pid_file).await;
    let mut cleanup = TestProcessCleanup::new(vec![leader_pid, descendant_pid]);

    assert!(result.is_error, "{}", result.output);
    let envelope: serde_json::Value = serde_json::from_str(&result.output).expect("tool JSON");
    assert_eq!(envelope["error"]["code"].as_str(), Some("timeout"));
    let pids = [leader_pid, descendant_pid];
    let exited = wait_for_processes_to_exit(&pids).await;
    if exited {
        cleanup.disarm();
    }
    assert!(
        exited,
        "timeout left foreground process tree alive: leader={leader_pid}, descendant={descendant_pid}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_foreground_execution_kills_entire_process_group() {
    let temp = tempfile::tempdir().expect("tempdir");
    let working_dir = temp.path().canonicalize().expect("canonical tempdir");
    let pid_file_name = "cancel-process-tree.pids";
    let pid_file = working_dir.join(pid_file_name);
    let command = foreground_process_tree_command(&working_dir, pid_file_name);

    let execution = tokio::spawn(execute_foreground(
        command,
        Duration::from_secs(60),
        None,
        working_dir.join("cancel-output.log"),
    ));
    let (leader_pid, descendant_pid) = wait_for_pid_file(&pid_file).await;
    let mut cleanup = TestProcessCleanup::new(vec![leader_pid, descendant_pid]);
    assert!(process_is_alive(leader_pid));
    assert!(process_is_alive(descendant_pid));

    execution.abort();
    let join_error = execution
        .await
        .expect_err("aborted foreground execution should be cancelled");
    assert!(join_error.is_cancelled());

    let pids = [leader_pid, descendant_pid];
    let exited = wait_for_processes_to_exit(&pids).await;
    if exited {
        cleanup.disarm();
    }
    assert!(
        exited,
        "dropping foreground execution left process tree alive: leader={leader_pid}, descendant={descendant_pid}"
    );
}
