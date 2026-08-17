//! `mitsuro update` — check or apply a coordinated release, then relaunch.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use mitsuro_core::updater::{
    apply_managed_release_update, check_for_updates, UpdateApplyPolicy, UpdateChannel,
    UpdateStatus, VERSION,
};
use tokio::sync::mpsc;

pub async fn run(apply: bool, version: Option<String>) -> Result<()> {
    if apply {
        apply_and_relaunch(version.as_deref()).await
    } else {
        print_check().await
    }
}

async fn print_check() -> Result<()> {
    match check_for_updates().await? {
        None => {
            println!("Mitsuro {VERSION} is up to date.");
            Ok(())
        }
        Some(info) => {
            println!(
                "Update available: {} → {}",
                info.current_version, info.new_version
            );
            if !info.release_notes.is_empty() {
                println!("{}", info.release_notes);
            }
            println!("{}", info.apply.guidance());
            Ok(())
        }
    }
}

pub async fn apply_and_relaunch(requested: Option<&str>) -> Result<()> {
    let channel = UpdateChannel::detect();
    match channel.apply_policy() {
        UpdateApplyPolicy::ManagedRelease => {}
        policy => anyhow::bail!("{}", policy.guidance()),
    }

    let version = match requested {
        Some(version) => version.trim_start_matches('v').to_string(),
        None => {
            let info = check_for_updates()
                .await?
                .ok_or_else(|| anyhow!("Mitsuro {VERSION} is already up to date."))?;
            if !info.apply.can_apply() {
                anyhow::bail!("{}", info.apply.guidance());
            }
            info.new_version
        }
    };

    println!("Updating Mitsuro {VERSION} → {version}");
    println!();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let apply = tokio::spawn(async move { apply_managed_release_update(&version, tx).await });

    while let Some(status) = rx.recv().await {
        match status {
            UpdateStatus::Downloading { progress, fraction } => draw_bar(&progress, fraction),
            UpdateStatus::Installing { progress } => draw_bar(&progress, None),
            UpdateStatus::Ready { version } => {
                draw_bar(&format!("Installed v{version}"), Some(1.0));
                println!();
            }
            UpdateStatus::Error(error) => {
                println!();
                anyhow::bail!("{error}");
            }
            _ => {}
        }
    }

    let relaunch = apply.await.context("update task failed")??;
    relaunch_mitsuro(&relaunch)
}

fn draw_bar(label: &str, fraction: Option<f32>) {
    const WIDTH: usize = 28;
    let filled = fraction
        .map(|value| ((value.clamp(0.0, 1.0) * WIDTH as f32).round() as usize).min(WIDTH))
        .unwrap_or(WIDTH / 3);
    let bar: String = std::iter::repeat_n('█', filled)
        .chain(std::iter::repeat_n('░', WIDTH.saturating_sub(filled)))
        .collect();
    let percent = fraction
        .map(|value| format!("{:>3}%", (value.clamp(0.0, 1.0) * 100.0).round() as u16))
        .unwrap_or_else(|| "    ".to_string());
    let mut stderr = io::stderr();
    let _ = write!(stderr, "\r  [{bar}] {percent}  {label:<40}");
    let _ = stderr.flush();
}

fn relaunch_mitsuro(command: &PathBuf) -> Result<()> {
    println!("Starting updated Mitsuro...");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(command).exec();
        Err(error).context(format!("failed to launch {}", command.display()))
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(command)
            .status()
            .with_context(|| format!("failed to launch {}", command.display()))?;
        Ok(())
    }
}
