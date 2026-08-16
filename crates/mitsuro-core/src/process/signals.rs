use anyhow::Context;

#[cfg(unix)]
use anyhow::Result;

#[cfg(unix)]
fn platform_pid(pid: u32) -> Result<libc::pid_t> {
    let pid = libc::pid_t::try_from(pid)
        .with_context(|| format!("process ID {pid} does not fit the platform pid type"))?;
    anyhow::ensure!(pid > 0, "invalid process ID {pid}");
    Ok(pid)
}

/// Signal only the process group led by `pid`.
///
/// This deliberately does not fall back to the individual process. Callers
/// that are cleaning up descendants after the group leader exits need to know
/// whether the group signal itself succeeded.
#[cfg(unix)]
pub(crate) fn signal_process_group(pid: u32, signal: libc::c_int, signal_name: &str) -> Result<()> {
    let pid = platform_pid(pid)?;
    if unsafe { libc::kill(-pid, signal) } == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    anyhow::bail!("failed to send {signal_name} to process group {pid}: {error}")
}

/// Return whether the process group led by `pid` still has any members.
#[cfg(unix)]
pub(crate) fn process_group_exists(pid: u32) -> Result<bool> {
    let pid = platform_pid(pid)?;
    if unsafe { libc::kill(-pid, 0) } == 0 {
        return Ok(true);
    }

    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        // A permission failure still proves that the group exists.
        Some(libc::EPERM) => Ok(true),
        _ => Err(error).with_context(|| format!("failed to inspect process group {pid}")),
    }
}

#[cfg(unix)]
pub(crate) fn signal_process(pid: u32, signal: libc::c_int, signal_name: &str) -> Result<()> {
    let pid = platform_pid(pid)?;
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    anyhow::bail!("failed to send {signal_name} to process {pid}: {error}")
}

/// Snapshot every Linux descendant of `root_pid`, including processes that
/// created their own session/process group inside a sandbox wrapper.
#[cfg(target_os = "linux")]
pub(crate) fn descendant_processes(root_pid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut parent_by_pid = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let Some(parent) = status.lines().find_map(|line| {
            line.strip_prefix("PPid:")
                .and_then(|value| value.trim().parse::<u32>().ok())
        }) else {
            continue;
        };
        parent_by_pid.push((pid, parent));
    }

    let mut descendants = Vec::new();
    let mut frontier = vec![root_pid];
    while let Some(parent) = frontier.pop() {
        for (pid, candidate_parent) in &parent_by_pid {
            if *candidate_parent == parent && !descendants.contains(pid) {
                descendants.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    descendants
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn descendant_processes(_root_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// Signal a registry-owned process group, falling back to the individual PID
/// for externally registered processes that are not process-group leaders.
#[cfg(unix)]
pub(crate) fn signal_process_or_group(
    pid: u32,
    signal: libc::c_int,
    signal_name: &str,
) -> Result<()> {
    let group_error = match signal_process_group(pid, signal, signal_name) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    let process_error = match signal_process(pid, signal, signal_name) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    anyhow::bail!("{group_error:#}; fallback to process {pid} also failed: {process_error:#}")
}

#[cfg(unix)]
fn signal_process_tree(pid: u32, signal: libc::c_int, signal_name: &str) -> Result<()> {
    // Capture descendants before signaling the outer wrapper. Sandboxes such
    // as bubblewrap may create a nested session; after the wrapper exits those
    // children are reparented and are no longer discoverable from `pid`.
    let descendants = descendant_processes(pid);
    let root_result = signal_process_or_group(pid, signal, signal_name);
    for descendant in descendants.into_iter().rev() {
        if let Err(error) = signal_process(descendant, signal, signal_name) {
            // A sibling/group signal may already have ended this process.
            // Preserve the authoritative root delivery result while retaining
            // diagnostics for a genuinely unresponsive descendant.
            tracing::debug!(
                root_pid = pid,
                descendant_pid = descendant,
                %error,
                "Could not signal captured process descendant"
            );
        }
    }
    root_result
}

#[cfg(unix)]
pub(super) fn terminate_process_tree(pid: u32) -> Result<()> {
    signal_process_tree(pid, libc::SIGTERM, "SIGTERM")
}

#[cfg(windows)]
pub(super) fn terminate_process_tree(pid: u32) -> anyhow::Result<()> {
    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .with_context(|| format!("failed to launch taskkill for process {pid}"))?;
    anyhow::ensure!(
        output.status.success(),
        "taskkill failed for process {pid}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

#[cfg(unix)]
pub(super) fn suspend_process_tree(pid: u32) -> anyhow::Result<()> {
    signal_process_tree(pid, libc::SIGSTOP, "SIGSTOP")
}

#[cfg(windows)]
pub(super) fn suspend_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Suspend not supported on Windows");
}

#[cfg(unix)]
pub(super) fn resume_process_tree(pid: u32) -> anyhow::Result<()> {
    signal_process_tree(pid, libc::SIGCONT, "SIGCONT")
}

#[cfg(windows)]
pub(super) fn resume_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Resume not supported on Windows");
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::process::CommandExt;
    use std::time::Duration;

    use super::{
        process_group_exists, resume_process_tree, signal_process, signal_process_group,
        suspend_process_tree, terminate_process_tree,
    };

    #[test]
    fn strict_group_signal_and_liveness_track_a_real_process_group() {
        let mut command = std::process::Command::new("sleep");
        command.arg("30").process_group(0);
        let mut child = command.spawn().expect("spawn process-group leader");
        let pid = child.id();

        assert!(process_group_exists(pid).expect("inspect live process group"));
        if let Err(error) = signal_process_group(pid, libc::SIGTERM, "SIGTERM") {
            let _ = child.kill();
            let _ = child.wait();
            panic!("strict process-group signal should succeed: {error:#}");
        }
        let status = child.wait().expect("reap terminated group leader");
        assert!(
            !status.success(),
            "SIGTERM should terminate the group leader"
        );
        assert!(!process_group_exists(pid).expect("inspect terminated process group"));
    }

    #[test]
    fn falls_back_to_signaling_a_process_that_is_not_a_group_leader() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep process");
        let pid = child.id();

        if let Err(error) = terminate_process_tree(pid) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("process-ID fallback should deliver SIGTERM: {error:#}");
        }

        let status = child.wait().expect("reap terminated child");
        assert!(!status.success(), "SIGTERM should terminate the child");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_tree_signal_reaches_descendant_in_a_nested_session() {
        let temp = tempfile::TempDir::new().expect("temp directory");
        let pid_file = temp.path().join("nested.pid");
        let mut command = std::process::Command::new("sh");
        command
            .arg("-c")
            .arg("setsid sleep 30 & echo $! > \"$NESTED_PID_FILE\"; wait")
            .env("NESTED_PID_FILE", &pid_file)
            .process_group(0);
        let mut wrapper = command.spawn().expect("spawn process wrapper");
        let wrapper_pid = wrapper.id();

        let nested_pid = (0..100)
            .find_map(|_| {
                let parsed = std::fs::read_to_string(&pid_file)
                    .ok()
                    .and_then(|value| value.trim().parse::<u32>().ok());
                if parsed.is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                parsed
            })
            .unwrap_or_else(|| {
                let _ = wrapper.kill();
                let _ = wrapper.wait();
                panic!("nested process PID was not published")
            });

        if let Err(error) = terminate_process_tree(wrapper_pid) {
            let _ = wrapper.kill();
            let _ = wrapper.wait();
            let _ = signal_process(nested_pid, libc::SIGKILL, "SIGKILL");
            panic!("tree termination should reach the wrapper: {error:#}");
        }
        let _ = wrapper.wait();

        let nested_stopped = (0..100).any(|_| {
            let stopped =
                std::fs::read_to_string(format!("/proc/{nested_pid}/stat")).map_or(true, |stat| {
                    stat.rsplit_once(") ")
                        .and_then(|(_, fields)| fields.chars().next())
                        == Some('Z')
                });
            if !stopped {
                std::thread::sleep(Duration::from_millis(10));
            }
            stopped
        });
        if !nested_stopped {
            let _ = signal_process(nested_pid, libc::SIGKILL, "SIGKILL");
        }
        assert!(
            nested_stopped,
            "nested-session descendant {nested_pid} survived registry tree termination"
        );
    }

    #[test]
    fn signal_failure_is_reported_instead_of_treated_as_success() {
        let pid = u32::MAX;

        for result in [
            suspend_process_tree(pid),
            resume_process_tree(pid),
            terminate_process_tree(pid),
        ] {
            let error = result.expect_err("an invalid process ID must fail");
            assert!(
                error.to_string().contains("does not fit"),
                "unexpected error: {error:#}"
            );
        }
    }
}
