#[cfg(unix)]
use anyhow::anyhow;
use anyhow::Result;

#[cfg(unix)]
const UNIX_MANAGED_UPDATE_MESSAGE: &str = "Automatic updates are disabled on Unix because the krusty compatibility binary, krusty-mako Hive service, and their service units must be upgraded together. Upgrade the complete Mitsuro release with your package manager (Homebrew/AUR), or rerun the checksum-verifying installer from https://github.com/honeycomb-Technologies/Mitsuro/blob/main/install.sh.";

#[cfg(unix)]
pub(super) fn require_safe_single_binary_update() -> Result<()> {
    Err(anyhow!(UNIX_MANAGED_UPDATE_MESSAGE))
}

#[cfg(not(unix))]
pub(super) fn require_safe_single_binary_update() -> Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn self_update_guidance() -> Option<&'static str> {
    Some(UNIX_MANAGED_UPDATE_MESSAGE)
}

#[cfg(not(unix))]
pub fn self_update_guidance() -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn unix_single_binary_updates_fail_closed_with_actionable_guidance() {
        let error = require_safe_single_binary_update().expect_err("Unix must fail closed");
        let message = error.to_string();

        assert!(message.contains("krusty-mako"));
        assert!(message.contains("service units"));
        assert!(message.contains("Homebrew/AUR"));
        assert!(message.contains("install.sh"));
        assert!(message.contains("checksum-verifying"));
        assert_eq!(self_update_guidance(), Some(message.as_str()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_single_binary_updates_remain_supported() {
        require_safe_single_binary_update().expect("Windows may update the standalone binary");
    }
}
