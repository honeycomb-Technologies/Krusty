use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use regex::Regex;

type PrCacheValue = (Instant, Option<u64>);
type PrCache = HashMap<String, PrCacheValue>;

static PR_CACHE: Lazy<Mutex<PrCache>> = Lazy::new(|| Mutex::new(HashMap::new()));
static GH_AVAILABLE: Lazy<bool> = Lazy::new(|| {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
});
const PR_CACHE_TTL: Duration = Duration::from_secs(60);
const PR_CACHE_MAX_ENTRIES: usize = 1024;

pub(super) fn resolve_pr_number(repo_root: &Path, branch: Option<&str>) -> Option<u64> {
    let branch = branch.unwrap_or_default();
    let cache_key = format!("{}::{}", repo_root.display(), branch);
    let now = Instant::now();

    if let Ok(mut cache) = PR_CACHE.lock() {
        if let Some((timestamp, value)) = cache.get(&cache_key) {
            if now.duration_since(*timestamp) < PR_CACHE_TTL {
                return *value;
            }
        }
        cache.remove(&cache_key);
    }

    let resolved =
        extract_pr_from_branch_name(branch).or_else(|| query_pr_number_from_gh(repo_root));

    if let Ok(mut cache) = PR_CACHE.lock() {
        if cache.len() >= PR_CACHE_MAX_ENTRIES {
            prune_pr_cache(&mut cache, now);
        }
        cache.insert(cache_key, (now, resolved));
    }

    resolved
}

fn prune_pr_cache(cache: &mut PrCache, now: Instant) {
    cache.retain(|_, (timestamp, _)| now.duration_since(*timestamp) < PR_CACHE_TTL);

    while cache.len() >= PR_CACHE_MAX_ENTRIES {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, (timestamp, _))| *timestamp)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

pub(super) fn extract_pr_from_branch_name(branch: &str) -> Option<u64> {
    if branch.trim().is_empty() {
        return None;
    }

    static PR_BRANCH_PATTERN: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(?:^|/)(?:pr|pull|pull-request)[/-]?(\d+)$")
            .expect("valid PR branch regex")
    });

    PR_BRANCH_PATTERN
        .captures(branch)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<u64>().ok())
}

fn query_pr_number_from_gh(repo_root: &Path) -> Option<u64> {
    if !*GH_AVAILABLE {
        return None;
    }

    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number", "--jq", ".number"])
        .current_dir(repo_root)
        .env("GH_FORCE_TTY", "0")
        .env("NO_COLOR", "1")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
}
