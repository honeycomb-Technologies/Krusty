use tokio::process::Command;

use crate::process::CommandEnvironment;
use crate::tools::ToolContext;

/// Strip ANSI escape sequences from text.
pub(super) fn strip_ansi(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b\[[\?0-9;]*[a-zA-Z]")
        .unwrap_or_else(|error| panic!("valid ANSI regex: {error}"));
    re.replace_all(text, "").into_owned()
}

/// Detect a trailing shell background operator (`&`) that is not quoted/escaped,
/// and return the command without it.
pub(super) fn strip_shell_background_suffix(command: &str) -> Option<String> {
    let trimmed = command.trim_end();
    let (amp_idx, last_char) = trimmed.char_indices().last()?;
    if last_char != '&' {
        return None;
    }

    let prefix = trimmed[..amp_idx].trim_end();
    if prefix.is_empty() {
        return None;
    }

    if matches!(prefix.chars().last(), Some('&' | '|')) {
        return None;
    }

    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (idx, ch) in trimmed.char_indices() {
        if idx == amp_idx {
            if in_single || in_double || escaped {
                return None;
            }
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    Some(prefix.to_string())
}

/// Detect an unquoted shell background operator that is not the normalized
/// final suffix owned by `run_in_background`. Redirect forms such as `2>&1`,
/// `&>`, `|&`, and logical `&&` are not background jobs.
pub(super) fn contains_embedded_background_operator(command: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let characters = command.char_indices().collect::<Vec<_>>();
    for (position, (_, character)) in characters.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match *character {
            '\\' if !in_single => {
                escaped = true;
                continue;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                continue;
            }
            '"' if !in_single => {
                in_double = !in_double;
                continue;
            }
            '&' if !in_single && !in_double => {}
            _ => continue,
        }

        let previous = position.checked_sub(1).map(|index| characters[index].1);
        let next = characters
            .get(position + 1)
            .map(|(_, character)| *character);
        if matches!(previous, Some('&' | '>' | '<' | '|')) || matches!(next, Some('&' | '>')) {
            continue;
        }
        return true;
    }
    false
}

fn shell_index_is_unquoted(command: &str, target_index: usize) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (index, character) in command.char_indices() {
        if index >= target_index {
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match character {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    !in_single && !in_double && !escaped
}

fn strip_unquoted_discard_redirect(command: &str) -> Option<String> {
    // Keep this deliberately narrow. The process registry already owns output
    // capture, so common `/dev/null` wrappers are redundant, but arbitrary
    // redirects may be part of the command's intended behavior.
    const SUFFIXES: &[&str] = &[
        " 1> /dev/null 2>&1",
        " 1>/dev/null 2>&1",
        " > /dev/null 2>&1",
        " >/dev/null 2>&1",
    ];

    let trimmed = command.trim_end();
    for suffix in SUFFIXES {
        let Some(prefix) = trimmed.strip_suffix(suffix) else {
            continue;
        };
        let suffix_index = prefix.len();
        if !prefix.trim().is_empty() && shell_index_is_unquoted(trimmed, suffix_index) {
            return Some(prefix.trim_end().to_string());
        }
    }
    None
}

/// Canonicalize shell-owned detachment syntax before handing a process to the
/// registry. The registry is the sole owner of process lifetime and output, so
/// `nohup`, a final `&`, and terminal `/dev/null` redirects are redundant and
/// otherwise make an identical launch look like a different process.
///
/// Returns `(command, inferred_background, removed_detachment_wrapper)`.
pub(super) fn normalize_tracked_background_command(command: &str) -> (String, bool, bool) {
    let inferred_command = strip_shell_background_suffix(command);
    let inferred_background = inferred_command.is_some();
    let mut normalized = inferred_command.unwrap_or_else(|| command.trim().to_string());
    let mut removed_detachment_wrapper = false;

    loop {
        let trimmed = normalized.trim();
        if let Some(remainder) = trimmed.strip_prefix("nohup ") {
            normalized = remainder.trim_start().to_string();
            removed_detachment_wrapper = true;
            continue;
        }
        if let Some(command_without_redirect) = strip_unquoted_discard_redirect(trimmed) {
            normalized = command_without_redirect;
            removed_detachment_wrapper = true;
            continue;
        }
        break;
    }

    (
        normalized.trim().to_string(),
        inferred_background,
        removed_detachment_wrapper,
    )
}

pub(super) fn build_shell_command(command: &str, ctx: &ToolContext) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    command_environment(ctx).apply(&mut cmd);

    if let Some(ref identity) = ctx.git_identity {
        for (key, val) in identity.env_vars() {
            cmd.env(key, val);
        }
    }

    cmd.current_dir(&ctx.working_dir);
    cmd
}

pub(super) fn command_environment(ctx: &ToolContext) -> CommandEnvironment {
    let mut overrides = ctx.command_environment.clone();
    overrides.insert("NO_COLOR".to_string(), "1".to_string());
    CommandEnvironment::new(ctx.command_environment_policy, overrides)
        .with_project_cache_defaults(ctx.project_dir.as_deref())
}

pub(super) fn configure_foreground_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
}
