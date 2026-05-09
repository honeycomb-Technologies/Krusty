use once_cell::sync::Lazy;
use regex::Regex;

static FORK_BOMB_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:").unwrap());
static NETWORK_PIPE_TO_SHELL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(curl|wget)\b.*\|\s*(sh|bash)\b").unwrap());
static DANGEROUS_REDIRECT_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)>\s*/dev/(sd|nvme|vd|xvd|disk)").unwrap());

pub(crate) fn safety_violation(command: &str) -> Option<String> {
    if FORK_BOMB_PATTERN.is_match(command) {
        return Some("fork bomb".to_string());
    }
    if NETWORK_PIPE_TO_SHELL_PATTERN.is_match(command) {
        return Some("network script piped to shell".to_string());
    }
    if DANGEROUS_REDIRECT_PATTERN.is_match(command) {
        return Some("raw disk redirection".to_string());
    }

    for segment in split_shell_segments(command) {
        if let Some(reason) = dangerous_command_reason(&segment) {
            return Some(reason.to_string());
        }
    }

    None
}

pub(super) fn is_modifying_bash_command(command: &str) -> bool {
    split_shell_segments(command)
        .iter()
        .any(|segment| is_mutating_shell_segment(segment))
}

fn split_shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => {
                current.push(ch);
                escaped = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    segments.push(trimmed.to_string());
                }
                current.clear();
            }
            '|' | '&' if !in_single && !in_double => {
                if matches!(chars.peek(), Some(next) if *next == ch) {
                    let _ = chars.next();
                }
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    segments.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }

    segments
}

fn tokenize_shell(segment: &str) -> Vec<String> {
    shell_words::split(segment).unwrap_or_else(|_| {
        segment
            .split_whitespace()
            .map(ToString::to_string)
            .collect()
    })
}

fn is_env_assignment(token: &str) -> bool {
    let Some((key, _)) = token.split_once('=') else {
        return false;
    };
    !key.is_empty() && key.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn strip_env_prefix(tokens: &[String]) -> &[String] {
    let mut idx = 0;
    while idx < tokens.len() && is_env_assignment(&tokens[idx]) {
        idx += 1;
    }
    &tokens[idx..]
}

fn has_unquoted_redirect(segment: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in segment.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '>' if !in_single && !in_double => return true,
            _ => {}
        }
    }

    false
}

fn is_dangerous_rm(tokens: &[String]) -> bool {
    let has_force = tokens
        .iter()
        .skip(1)
        .any(|t| t.starts_with('-') && t.contains('f'));
    let has_recursive = tokens
        .iter()
        .skip(1)
        .any(|t| t.starts_with('-') && t.contains('r'));
    if !(has_force && has_recursive) {
        return false;
    }

    tokens
        .iter()
        .skip(1)
        .filter(|t| !t.starts_with('-'))
        .any(|target| {
            matches!(
                target.as_str(),
                "/" | "/*" | "~" | "~/" | "$HOME" | "$HOME/" | "${HOME}" | "${HOME}/"
            ) || target.starts_with("/etc")
                || target.starts_with("/usr")
                || target.starts_with("/var")
        })
}

fn dangerous_command_reason(segment: &str) -> Option<&'static str> {
    if FORK_BOMB_PATTERN.is_match(segment) {
        return Some("fork bomb");
    }
    if NETWORK_PIPE_TO_SHELL_PATTERN.is_match(segment) {
        return Some("network script piped to shell");
    }
    if DANGEROUS_REDIRECT_PATTERN.is_match(segment) {
        return Some("raw disk redirection");
    }

    let tokens = tokenize_shell(segment);
    let tokens = strip_env_prefix(&tokens);
    let command = tokens.first().map(|t| t.to_ascii_lowercase())?;

    if matches!(command.as_str(), "sudo" | "doas" | "su") {
        return Some("privilege escalation");
    }

    if command == "rm" && is_dangerous_rm(tokens) {
        return Some("destructive rm target");
    }

    if command == "chmod"
        && tokens
            .iter()
            .skip(1)
            .any(|t| matches!(t.as_str(), "777" | "0777"))
    {
        return Some("unsafe chmod 777");
    }

    if command == "dd"
        && tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with("of=/dev/") || t.starts_with("if=/dev/"))
    {
        return Some("direct disk access with dd");
    }

    if command.starts_with("mkfs") {
        return Some("filesystem formatting command");
    }

    None
}

fn is_mutating_git_subcommand(subcommand: Option<&str>) -> bool {
    !matches!(
        subcommand,
        Some("status")
            | Some("diff")
            | Some("show")
            | Some("log")
            | Some("grep")
            | Some("rev-parse")
            | Some("ls-files")
    )
}

fn is_mutating_shell_segment(segment: &str) -> bool {
    if has_unquoted_redirect(segment) {
        return true;
    }

    let tokens = tokenize_shell(segment);
    let tokens = strip_env_prefix(&tokens);
    let Some(command) = tokens.first().map(|t| t.to_ascii_lowercase()) else {
        return false;
    };

    if matches!(
        command.as_str(),
        "rm" | "rmdir"
            | "mkdir"
            | "mv"
            | "cp"
            | "touch"
            | "chmod"
            | "chown"
            | "ln"
            | "tee"
            | "dd"
            | "mkfs"
            | "truncate"
            | "install"
            | "tar"
            | "unzip"
            | "bun"
            | "npm"
            | "yarn"
            | "pip"
            | "cargo"
            | "make"
            | "cmake"
            | "ninja"
    ) {
        return true;
    }

    if command == "git" {
        let subcommand = tokens.get(1).map(|s| s.to_ascii_lowercase());
        return is_mutating_git_subcommand(subcommand.as_deref());
    }

    false
}
