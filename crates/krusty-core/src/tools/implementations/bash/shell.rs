use tokio::process::Command;

use crate::tools::ToolContext;

/// Keep the tail of a string within `max_bytes`, preserving UTF-8 boundaries.
fn tail_by_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Strip ANSI escape sequences from text.
pub(super) fn strip_ansi(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b\[[\?0-9;]*[a-zA-Z]")
        .expect("valid regex");
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

    cmd.env("NO_COLOR", "1");

    if let Some(ref identity) = ctx.git_identity {
        for (key, val) in identity.env_vars() {
            cmd.env(key, val);
        }
    }

    cmd.current_dir(&ctx.working_dir);
    cmd
}

pub(super) fn configure_foreground_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
}

#[allow(dead_code)]
fn _tail_by_bytes_for_tests(text: &str, max_bytes: usize) -> String {
    tail_by_bytes(text, max_bytes)
}
