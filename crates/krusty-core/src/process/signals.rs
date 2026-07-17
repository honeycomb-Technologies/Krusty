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
fn signal_process(pid: u32, signal: libc::c_int, signal_name: &str) -> Result<()> {
    let pid = platform_pid(pid)?;
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    anyhow::bail!("failed to send {signal_name} to process {pid}: {error}")
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
pub(super) fn terminate_process_tree(pid: u32) -> Result<()> {
    signal_process_or_group(pid, libc::SIGTERM, "SIGTERM")
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
    signal_process_or_group(pid, libc::SIGSTOP, "SIGSTOP")
}

#[cfg(windows)]
pub(super) fn suspend_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Suspend not supported on Windows");
}

#[cfg(unix)]
pub(super) fn resume_process_tree(pid: u32) -> anyhow::Result<()> {
    signal_process_or_group(pid, libc::SIGCONT, "SIGCONT")
}

#[cfg(windows)]
pub(super) fn resume_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Resume not supported on Windows");
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::process::CommandExt;

    use super::{
        process_group_exists, resume_process_tree, signal_process_group, suspend_process_tree,
        terminate_process_tree,
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
