use anyhow::anyhow;
use anyhow::Result;

pub(crate) const MANAGED_UPDATE_MESSAGE: &str = "Automatic in-app updates are disabled because the Mitsuro CLI, Hive service, compatibility shims, and service integration must be upgraded together. Upgrade the complete Mitsuro release with your platform package manager, or rerun the checksum-verifying installer from https://github.com/honeycomb-Technologies/Mitsuro/blob/main/install.sh.";

pub(super) fn require_safe_single_binary_update() -> Result<()> {
    Err(anyhow!(MANAGED_UPDATE_MESSAGE))
}

pub fn self_update_guidance() -> Option<&'static str> {
    Some(MANAGED_UPDATE_MESSAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_binary_updates_fail_closed_with_actionable_guidance_on_every_platform() {
        let error = require_safe_single_binary_update().expect_err("updater must fail closed");
        let message = error.to_string();

        assert!(message.contains("Hive service"));
        assert!(message.contains("compatibility shims"));
        assert!(message.contains("platform package manager"));
        assert!(message.contains("install.sh"));
        assert!(message.contains("checksum-verifying"));
        assert_eq!(self_update_guidance(), Some(message.as_str()));
    }
}
