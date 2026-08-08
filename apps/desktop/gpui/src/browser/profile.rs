//! Chrome / Chromium profile discovery (paths only — no secrets).

use std::fs;
use std::path::{Path, PathBuf};

/// A single user-data profile directory under Chrome or Chromium.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredProfile {
    /// `"Chrome"` or `"Chromium"`.
    pub browser: String,
    /// Directory name (`Default`, `Profile 1`, …).
    pub name: String,
    /// Absolute path to the profile directory.
    pub path: PathBuf,
}

/// Result of scanning well-known Linux config roots.
#[derive(Clone, Debug, Default)]
pub struct ProfileDiscovery {
    pub profiles: Vec<DiscoveredProfile>,
    /// Roots that existed on disk (even if empty of profiles).
    pub roots_found: Vec<PathBuf>,
    /// Roots that were missing.
    pub roots_missing: Vec<PathBuf>,
}

impl ProfileDiscovery {
    pub fn count(&self) -> usize {
        self.profiles.len()
    }

    pub fn summary_label(&self) -> String {
        let n = self.count();
        if n == 0 {
            if self.roots_found.is_empty() {
                "No Chrome/Chromium config dirs found".into()
            } else {
                format!("0 profiles under {} root(s)", self.roots_found.len())
            }
        } else {
            let browsers: Vec<&str> = {
                let mut v: Vec<&str> = self.profiles.iter().map(|p| p.browser.as_str()).collect();
                v.sort_unstable();
                v.dedup();
                v
            };
            format!("{n} profile(s) · {}", browsers.join(" + "))
        }
    }
}

/// Scan `~/.config/google-chrome` and `~/.config/chromium` for profile dirs.
///
/// **Does not** open Cookies, Login Data, or other secret stores. A directory
/// counts as a profile if it contains a `Preferences` file (existence check only).
pub fn discover_browser_profiles() -> ProfileDiscovery {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => {
            return ProfileDiscovery::default();
        }
    };

    let roots: [(&str, PathBuf); 2] = [
        ("Chrome", home.join(".config/google-chrome")),
        ("Chromium", home.join(".config/chromium")),
    ];

    let mut out = ProfileDiscovery::default();

    for (browser, root) in roots {
        if root.is_dir() {
            out.roots_found.push(root.clone());
            out.profiles.extend(scan_user_data_dir(browser, &root));
        } else {
            out.roots_missing.push(root);
        }
    }

    out.profiles
        .sort_by(|a, b| a.browser.cmp(&b.browser).then_with(|| a.name.cmp(&b.name)));

    out
}

fn scan_user_data_dir(browser: &str, root: &Path) -> Vec<DiscoveredProfile> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if !looks_like_profile_name(&name) {
            continue;
        }
        // Existence only — do not read Preferences contents.
        if !path.join("Preferences").is_file() {
            continue;
        }
        found.push(DiscoveredProfile {
            browser: browser.to_string(),
            name,
            path,
        });
    }
    found
}

fn looks_like_profile_name(name: &str) -> bool {
    if name == "Default" || name == "Guest Profile" || name == "System Profile" {
        return true;
    }
    // "Profile 1", "Profile 2", …
    if let Some(rest) = name.strip_prefix("Profile ") {
        return rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_filter() {
        assert!(looks_like_profile_name("Default"));
        assert!(looks_like_profile_name("Profile 1"));
        assert!(looks_like_profile_name("Profile 12"));
        assert!(looks_like_profile_name("Guest Profile"));
        assert!(!looks_like_profile_name("ShaderCache"));
        assert!(!looks_like_profile_name("Crash Reports"));
        assert!(!looks_like_profile_name("Profile x"));
    }

    #[test]
    fn discover_runs_without_panic() {
        let d = discover_browser_profiles();
        // On this builder host both Chrome and Chromium often exist; count is best-effort.
        let _ = d.count();
        let label = d.summary_label();
        assert!(!label.is_empty());
    }
}
