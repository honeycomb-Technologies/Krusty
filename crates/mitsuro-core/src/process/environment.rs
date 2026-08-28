use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use tokio::process::Command;

/// Controls whether a child command inherits the Mitsuro process environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandEnvironmentPolicy {
    /// Preserve the historical behavior for direct, non-delegated commands.
    #[default]
    Inherit,
    /// Start from an empty environment and restore allowlisted host values plus
    /// explicitly safe runtime overrides.
    Sanitized,
    /// Start from an empty environment and restore only explicitly safe runtime
    /// overrides. No host environment value is inherited.
    Explicit,
}

/// Environment contract shared by foreground and tracked background commands.
#[derive(Debug, Clone)]
pub struct CommandEnvironment {
    policy: CommandEnvironmentPolicy,
    overrides: BTreeMap<String, String>,
}

const SAFE_INHERITED_KEYS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "ComSpec",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "COLORTERM",
    "TZ",
    "USER",
    "LOGNAME",
    "RUSTUP_HOME",
    "JAVA_HOME",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "DEVELOPER_DIR",
    "SDKROOT",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "GIT_SSL_CAINFO",
    "CARGO_HTTP_CAINFO",
    "NODE_EXTRA_CA_CERTS",
];

const SANITIZED_OVERRIDE_KEYS: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "npm_config_cache",
    "NPM_CONFIG_CACHE",
    "NO_COLOR",
];

const EXPLICIT_OVERRIDE_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "npm_config_cache",
    "NPM_CONFIG_CACHE",
    "NO_COLOR",
];

impl CommandEnvironment {
    pub fn new(policy: CommandEnvironmentPolicy, overrides: BTreeMap<String, String>) -> Self {
        Self { policy, overrides }
    }

    pub fn inherited() -> Self {
        Self::new(CommandEnvironmentPolicy::Inherit, BTreeMap::new())
    }

    pub fn policy(&self) -> CommandEnvironmentPolicy {
        self.policy
    }

    /// Give ordinary project commands writable, project-isolated package
    /// caches without changing the user's HOME or leaking cache files into the
    /// repository. Explicit runtime overrides always win.
    pub fn with_project_cache_defaults(mut self, project_dir: Option<&std::path::Path>) -> Self {
        if self.policy != CommandEnvironmentPolicy::Inherit {
            return self;
        }
        let Some(project_dir) = project_dir else {
            return self;
        };
        let canonical = project_dir
            .canonicalize()
            .unwrap_or_else(|_| project_dir.to_path_buf());
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_os_str().to_string_lossy().as_bytes());
        let identity = format!("{:x}", hasher.finalize());
        let root = std::env::temp_dir()
            .join("mitsuro-command-cache")
            .join(&identity[..16]);
        let cache = root.join("xdg");
        let npm = root.join("npm");
        if std::fs::create_dir_all(&cache).is_ok() && std::fs::create_dir_all(&npm).is_ok() {
            self.overrides
                .entry("XDG_CACHE_HOME".to_string())
                .or_insert_with(|| cache.display().to_string());
            self.overrides
                .entry("npm_config_cache".to_string())
                .or_insert_with(|| npm.display().to_string());
            self.overrides
                .entry("NPM_CONFIG_CACHE".to_string())
                .or_insert_with(|| npm.display().to_string());
        }
        self
    }

    fn override_variables(&self, allowlist: &[&str]) -> BTreeMap<String, String> {
        self.overrides
            .iter()
            .filter(|(key, _)| allowlist.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn explicit_variables(&self) -> BTreeMap<String, String> {
        self.override_variables(EXPLICIT_OVERRIDE_KEYS)
    }

    fn sanitized_variables(&self) -> BTreeMap<String, String> {
        let mut variables = SAFE_INHERITED_KEYS
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| ((*key).to_string(), value))
            })
            .collect::<BTreeMap<_, _>>();
        variables.extend(self.override_variables(SANITIZED_OVERRIDE_KEYS));
        variables
    }

    pub fn apply(&self, command: &mut Command) {
        match self.policy {
            CommandEnvironmentPolicy::Inherit => {
                command.envs(&self.overrides);
            }
            CommandEnvironmentPolicy::Sanitized => {
                command.env_clear();
                command.envs(self.sanitized_variables());
            }
            CommandEnvironmentPolicy::Explicit => {
                command.env_clear();
                command.envs(self.explicit_variables());
            }
        }
    }

    /// Stable, non-reversible identity used to keep differently governed
    /// background commands from being treated as the same launch.
    pub(crate) fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(match self.policy {
            CommandEnvironmentPolicy::Inherit => b"inherit".as_slice(),
            CommandEnvironmentPolicy::Sanitized => b"sanitized".as_slice(),
            CommandEnvironmentPolicy::Explicit => b"explicit".as_slice(),
        });
        let variables = match self.policy {
            CommandEnvironmentPolicy::Inherit => self.overrides.clone(),
            CommandEnvironmentPolicy::Sanitized => self.sanitized_variables(),
            CommandEnvironmentPolicy::Explicit => self.explicit_variables(),
        };
        for (key, value) in variables {
            hasher.update([0]);
            hasher.update(key.as_bytes());
            hasher.update([0]);
            hasher.update(value.as_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_environment_accepts_runtime_paths_and_rejects_sensitive_overrides() {
        let environment = CommandEnvironment::new(
            CommandEnvironmentPolicy::Sanitized,
            BTreeMap::from([
                (
                    "PATH".to_string(),
                    "/must-not-override-sanitized-path".to_string(),
                ),
                ("HOME".to_string(), "/isolated/home".to_string()),
                ("USER".to_string(), "must-not-override-user".to_string()),
                ("TMPDIR".to_string(), "/isolated/tmp".to_string()),
                (
                    "MITSURO_TEST_SECRET".to_string(),
                    "must-not-escape".to_string(),
                ),
            ]),
        );

        let variables = environment.sanitized_variables();
        assert_eq!(
            variables.get("HOME").map(String::as_str),
            Some("/isolated/home")
        );
        assert_eq!(
            variables.get("TMPDIR").map(String::as_str),
            Some("/isolated/tmp")
        );
        assert_ne!(
            variables.get("PATH").map(String::as_str),
            Some("/must-not-override-sanitized-path")
        );
        assert_ne!(
            variables.get("USER").map(String::as_str),
            Some("must-not-override-user")
        );
        assert!(!variables.contains_key("MITSURO_TEST_SECRET"));
    }

    #[test]
    fn policy_and_effective_environment_change_the_fingerprint() {
        let overrides = BTreeMap::from([("HOME".to_string(), "/isolated/home".to_string())]);
        let inherited =
            CommandEnvironment::new(CommandEnvironmentPolicy::Inherit, overrides.clone());
        let sanitized = CommandEnvironment::new(CommandEnvironmentPolicy::Sanitized, overrides);

        assert_ne!(inherited.fingerprint(), sanitized.fingerprint());
        assert_eq!(sanitized.policy(), CommandEnvironmentPolicy::Sanitized);
    }

    #[test]
    fn path_override_is_available_only_to_explicit_policy() {
        let supplied_path = "/runtime/explicit/bin";
        let overrides = BTreeMap::from([("PATH".to_string(), supplied_path.to_string())]);
        let sanitized =
            CommandEnvironment::new(CommandEnvironmentPolicy::Sanitized, overrides.clone());
        let explicit = CommandEnvironment::new(CommandEnvironmentPolicy::Explicit, overrides);

        assert_ne!(
            sanitized
                .sanitized_variables()
                .get("PATH")
                .map(String::as_str),
            Some(supplied_path),
            "Sanitized must preserve its historical override allowlist"
        );
        assert_eq!(
            explicit
                .explicit_variables()
                .get("PATH")
                .map(String::as_str),
            Some(supplied_path),
            "Explicit is the Worker Goal-only deterministic environment policy"
        );
    }

    #[test]
    fn inherited_project_commands_receive_writable_scoped_package_caches() {
        let project = tempfile::tempdir().expect("project");
        let environment =
            CommandEnvironment::inherited().with_project_cache_defaults(Some(project.path()));

        let npm = environment
            .overrides
            .get("npm_config_cache")
            .expect("npm cache");
        assert!(std::path::Path::new(npm).is_dir());
        assert_eq!(environment.overrides.get("NPM_CONFIG_CACHE"), Some(npm));
        assert!(environment.overrides.contains_key("XDG_CACHE_HOME"));
        assert!(!environment.overrides.contains_key("HOME"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn applying_sanitized_policy_clears_a_preconfigured_sensitive_value() {
        let environment = CommandEnvironment::new(
            CommandEnvironmentPolicy::Sanitized,
            BTreeMap::from([("HOME".to_string(), "/isolated/home".to_string())]),
        );
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("printf '%s|%s' \"${MITSURO_TEST_SECRET-unset}\" \"$HOME\"")
            .env("MITSURO_TEST_SECRET", "must-not-escape");

        environment.apply(&mut command);
        let output = command.output().await.expect("run sanitized command");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8 output"),
            "unset|/isolated/home"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_policy_does_not_inherit_allowlisted_host_values() {
        let environment = CommandEnvironment::new(
            CommandEnvironmentPolicy::Explicit,
            BTreeMap::from([
                ("PATH".to_string(), "/usr/bin:/bin".to_string()),
                ("HOME".to_string(), "/isolated/home".to_string()),
            ]),
        );
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("printf '%s|%s' \"${JAVA_HOME-unset}\" \"$HOME\"")
            .env("JAVA_HOME", "/host/private/jdk");

        environment.apply(&mut command);
        let output = command.output().await.expect("run explicit command");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8 output"),
            "unset|/isolated/home"
        );
    }
}
