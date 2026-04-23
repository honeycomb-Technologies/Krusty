#[cfg(unix)]
pub(super) fn terminate_process_tree(pid: u32) {
    let pgid = format!("-{}", pid);
    let result = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(&pgid)
        .output();

    if result.is_err() {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output();
    }
}

#[cfg(windows)]
pub(super) fn terminate_process_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
pub(super) fn suspend_process_tree(pid: u32) -> anyhow::Result<()> {
    let pgid = format!("-{}", pid);
    let result = std::process::Command::new("kill")
        .arg("-STOP")
        .arg(&pgid)
        .output();

    if result.is_err() {
        let _ = std::process::Command::new("kill")
            .arg("-STOP")
            .arg(pid.to_string())
            .output();
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn suspend_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Suspend not supported on Windows");
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
pub(super) fn resume_process_tree(pid: u32) -> anyhow::Result<()> {
    let pgid = format!("-{}", pid);
    let result = std::process::Command::new("kill")
        .arg("-CONT")
        .arg(&pgid)
        .output();

    if result.is_err() {
        let _ = std::process::Command::new("kill")
            .arg("-CONT")
            .arg(pid.to_string())
            .output();
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn resume_process_tree(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Resume not supported on Windows");
}
