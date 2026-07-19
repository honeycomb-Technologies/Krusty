use once_cell::sync::Lazy;
use regex::Regex;

static FORK_BOMB_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:").unwrap());
static NETWORK_PIPE_TO_SHELL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(curl|wget)\b.*\|\s*(sh|bash)\b").unwrap());
static DANGEROUS_REDIRECT_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)>\s*/dev/(sd|nvme|vd|xvd|disk)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BashFileOperationKind {
    Read,
    Search,
    Edit,
}

impl BashFileOperationKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Read => "file read",
            Self::Search => "file search",
            Self::Edit => "file edit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BashFileOperation {
    pub(crate) kind: BashFileOperationKind,
    pub(crate) command: String,
    pub(crate) segment: String,
    pub(crate) recommended_tool: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BashCommandClassification {
    pub(crate) safety_violation: Option<String>,
    pub(crate) file_operation: Option<BashFileOperation>,
    pub(crate) modifies_filesystem_or_process: bool,
}

pub(crate) fn classify_bash_command(command: &str) -> BashCommandClassification {
    let segments = split_shell_segments(command);

    let safety_violation = if FORK_BOMB_PATTERN.is_match(command) {
        Some("fork bomb".to_string())
    } else if NETWORK_PIPE_TO_SHELL_PATTERN.is_match(command) {
        Some("network script piped to shell".to_string())
    } else if DANGEROUS_REDIRECT_PATTERN.is_match(command) {
        Some("raw disk redirection".to_string())
    } else {
        segments
            .iter()
            .find_map(|segment| dangerous_command_reason(segment).map(str::to_string))
    };

    let file_operation = segments
        .iter()
        .find_map(|segment| classify_file_operation_segment(segment));
    // Treat an unproven shell surface as effectful. A mutation blacklist
    // cannot safely classify interpreters, scripts, uncommon tools, shell
    // expansions, or environment-driven commands as observations.
    let modifies_filesystem_or_process = !is_proven_read_only_bash_command(command);

    BashCommandClassification {
        safety_violation,
        file_operation,
        modifies_filesystem_or_process,
    }
}

/// Build a stable, privacy-preserving-enough semantic key for progress
/// detection. Presentation-only differences (line numbers, color, output
/// caps, descriptions supplied outside the command, and harmless context
/// probes such as `pwd`) must not let a model evade the no-progress guard.
pub(crate) fn semantic_bash_signature(command: &str) -> String {
    let mut normalized_segments = split_shell_segments(command)
        .into_iter()
        .filter_map(|segment| normalize_progress_segment(&segment))
        .collect::<Vec<_>>();
    normalized_segments.sort();
    normalized_segments.dedup();

    if normalized_segments.is_empty() {
        "noop".to_string()
    } else {
        normalized_segments.join("|")
    }
}

fn normalize_progress_segment(segment: &str) -> Option<String> {
    let tokens = tokenize_shell(segment);
    let tokens = strip_invocation_wrappers(&tokens);
    let command = command_basename(tokens.first()?);

    // These commands add no durable evidence and frequently get prepended to
    // an otherwise identical search by models trying the same strategy again.
    if matches!(
        command.as_str(),
        "pwd" | "true" | "echo" | "printf" | "whoami" | "date"
    ) {
        return None;
    }
    if command == "cd" && tokens.get(1).is_some_and(|path| path == ".") {
        return None;
    }

    // A pipe-only output cap changes presentation, not investigative intent.
    if matches!(command.as_str(), "head" | "tail") && !head_or_tail_has_file_operand(tokens) {
        return None;
    }

    let mut normalized = vec![command.clone()];
    let mut index = 1;
    while index < tokens.len() {
        let token = tokens[index].as_str();

        let presentation_flag = matches!(
            token,
            "-n" | "--line-number"
                | "--no-heading"
                | "--heading"
                | "--stats"
                | "--short"
                | "--porcelain"
                | "--color"
        ) || token.starts_with("--color=");
        if presentation_flag {
            if token == "--color" {
                index += 1;
            }
            index += 1;
            continue;
        }

        if matches!(token, "-m" | "--max-count") {
            index = (index + 2).min(tokens.len());
            continue;
        }
        if token.starts_with("--max-count=") {
            index += 1;
            continue;
        }

        // `ls -la` versus `ls -al`, or a status formatting flag, does not
        // represent a different resource or strategy.
        if matches!(command.as_str(), "ls" | "tree") && token.starts_with('-') {
            index += 1;
            continue;
        }
        if command == "git"
            && tokens.get(1).map(String::as_str) == Some("status")
            && index > 1
            && token.starts_with('-')
        {
            index += 1;
            continue;
        }

        normalized.push(normalize_progress_token(token));
        index += 1;
    }

    Some(normalized.join(" "))
}

fn normalize_progress_token(token: &str) -> String {
    let without_dot = token.strip_prefix("./").unwrap_or(token);
    if without_dot.len() > 1 {
        without_dot.trim_end_matches('/').to_string()
    } else {
        without_dot.to_string()
    }
}

/// Free-form shell is never a Plan-mode capability. Even apparently
/// observational programs can write or execute through version-specific
/// flags, repository configuration, pagers/preprocessors, signals, PATH
/// resolution, or metadata refreshes. Plan mode already advertises native
/// Read/Glob/Grep and read-only delegated tools; shell returns only after the
/// user explicitly enters Build mode. Future Git/system inspection should be
/// exposed as typed argv APIs rather than another shell allowlist.
pub(crate) fn is_write_capable_in_plan_mode(_command: &str) -> bool {
    true
}

/// Return true only for the small audited shell surface that is proven
/// observational for progress accounting. Plan-mode governance is stricter
/// and denies free-form shell entirely.
pub(crate) fn is_proven_read_only_bash_command(command: &str) -> bool {
    // The lightweight segment parser intentionally does not parse heredoc
    // delimiters or newline command boundaries. Multiline shell is therefore
    // effectful by default; otherwise `echo ok\ntouch out` would be mistaken
    // for one harmless `echo` invocation.
    if command.contains('\n')
        || command.contains('\r')
        || command.contains("$(")
        || command.contains('`')
        || command.contains("<(")
        || command.contains(">(")
        || command.contains('$')
    {
        return false;
    }

    let segments = split_shell_segments(command);
    !segments.is_empty()
        && segments
            .iter()
            .all(|segment| is_plan_mode_read_only_segment(segment))
}

fn is_plan_mode_read_only_segment(segment: &str) -> bool {
    if has_unquoted_redirect(segment) {
        return false;
    }

    let raw_tokens = tokenize_shell(segment);
    if raw_tokens
        .first()
        .is_some_and(|token| is_env_assignment(token))
        || raw_tokens
            .iter()
            .take(4)
            .any(|token| command_basename(token) == "env")
    {
        return false;
    }
    let tokens = strip_invocation_wrappers(&raw_tokens);
    let Some(executable) = tokens.first() else {
        return false;
    };
    if executable.contains('/') || executable.contains('\\') {
        return false;
    }
    if executable != &executable.to_ascii_lowercase() {
        return false;
    }
    let command = command_basename(executable);

    if command == "cd" {
        return tokens.len() == 2 && tokens.get(1).is_some_and(|path| path == ".");
    }

    match command.as_str() {
        "git" => is_plan_mode_read_only_git(tokens),
        "tree" => is_plan_mode_read_only_tree(tokens),
        "sort" => is_plan_mode_read_only_sort(tokens),
        "rg" => is_plan_mode_read_only_rg(tokens),
        "date" => is_plan_mode_read_only_date(tokens),
        "hostname" => tokens.len() == 1,
        "ss" => is_plan_mode_read_only_ss(tokens),
        "printf" => is_plan_mode_read_only_printf(tokens),
        "ls" | "cat" | "head" | "tail" | "wc" | "stat" | "grep" | "diff" | "cmp" | "cut" | "tr"
        | "column" | "jq" | "basename" | "dirname" | "realpath" | "readlink" | "pwd" | "du"
        | "df" | "ps" | "pgrep" | "netstat" | "lsof" | "which" | "whereis" | "type"
        | "printenv" | "whoami" | "id" | "uname" | "uptime" | "free" | "md5sum" | "sha256sum"
        | "shasum" | "echo" | "true" | "false" | "test" | "[" => true,
        _ => false,
    }
}

fn is_plan_mode_read_only_git(tokens: &[String]) -> bool {
    // Output redirection implemented by Git itself bypasses the shell-level
    // redirect check. External diff hooks are executable code, not an
    // observational surface, even for otherwise read-only subcommands.
    if tokens.iter().skip(2).any(|token| {
        matches!(
            token.as_str(),
            "--output" | "--ext-diff" | "--textconv" | "--open-files-in-pager" | "--show-signature"
        ) || token.starts_with("--output=")
            || token.starts_with("--open-files-in-pager=")
    }) {
        return false;
    }

    match tokens.get(1).map(String::as_str) {
        Some(
            "status" | "diff" | "show" | "log" | "grep" | "rev-parse" | "ls-files" | "describe"
            | "shortlog" | "name-rev" | "blame",
        ) => true,
        Some("branch") => tokens.len() == 2 || tokens[2..] == ["--show-current"],
        _ => false,
    }
}

fn is_plan_mode_read_only_rg(tokens: &[String]) -> bool {
    !tokens.iter().skip(1).any(|token| {
        matches!(token.as_str(), "--pre" | "--hostname-bin")
            || token.starts_with("--pre=")
            || token.starts_with("--hostname-bin=")
    })
}

fn is_plan_mode_read_only_tree(tokens: &[String]) -> bool {
    !tokens.iter().skip(1).any(|token| {
        token == "-o"
            || token.starts_with("-o")
            || token == "--output"
            || token.starts_with("--output=")
    })
}

fn is_plan_mode_read_only_sort(tokens: &[String]) -> bool {
    !tokens.iter().skip(1).any(|token| {
        token == "-o"
            || token.starts_with("-o")
            || token == "--output"
            || token.starts_with("--output=")
            || token == "-T"
            || token.starts_with("-T")
            || token == "--temporary-directory"
            || token.starts_with("--temporary-directory=")
            || token == "--compress-program"
            || token.starts_with("--compress-program=")
    })
}

fn is_plan_mode_read_only_date(tokens: &[String]) -> bool {
    // BSD `date` accepts a bare numeric operand to set the clock; GNU `date`
    // exposes the same capability through -s/--set. Keep the allowlist small
    // and admit only display flags and +FORMAT operands.
    tokens.iter().skip(1).all(|token| {
        token.starts_with('+')
            || matches!(
                token.as_str(),
                "-u" | "--utc" | "-R" | "--rfc-email" | "--resolution" | "--help" | "--version"
            )
            || token == "-I"
            || token.starts_with("-I")
            || token == "--iso-8601"
            || token.starts_with("--iso-8601=")
            || token.starts_with("--rfc-3339=")
    })
}

fn is_plan_mode_read_only_ss(tokens: &[String]) -> bool {
    !tokens.iter().skip(1).any(|token| {
        token == "--kill"
            || (token.starts_with('-')
                && !token.starts_with("--")
                && token.chars().skip(1).any(|option| option == 'K'))
    })
}

fn is_plan_mode_read_only_printf(tokens: &[String]) -> bool {
    !tokens
        .iter()
        .skip(1)
        .any(|token| token.starts_with("-v") || token.starts_with("--variable="))
}

fn strip_invocation_wrappers(mut tokens: &[String]) -> &[String] {
    loop {
        tokens = strip_env_prefix(tokens);
        let Some(command) = tokens.first().map(|token| command_basename(token)) else {
            return tokens;
        };

        match command.as_str() {
            "command" | "builtin" | "noglob" => {
                tokens = &tokens[1..];
            }
            "env" => {
                let mut idx = 1;
                while idx < tokens.len() {
                    let token = tokens[idx].as_str();
                    if is_env_assignment(token) || matches!(token, "-i" | "--ignore-environment") {
                        idx += 1;
                    } else if matches!(token, "-u" | "--unset" | "-C" | "--chdir") {
                        idx = (idx + 2).min(tokens.len());
                    } else if token.starts_with('-') {
                        idx += 1;
                    } else {
                        break;
                    }
                }
                tokens = &tokens[idx..];
            }
            "timeout" | "gtimeout" => {
                let mut idx = 1;
                while idx < tokens.len() && tokens[idx].starts_with('-') {
                    idx += 1;
                }
                if idx < tokens.len() {
                    idx += 1;
                }
                tokens = &tokens[idx..];
            }
            _ => return tokens,
        }
    }
}

fn sed_in_place(tokens: &[String]) -> bool {
    tokens.iter().skip(1).any(|token| {
        matches!(token.as_str(), "-i" | "--in-place")
            || token.starts_with("-i")
            || token.starts_with("--in-place=")
    })
}

fn awk_in_place(tokens: &[String]) -> bool {
    tokens
        .windows(2)
        .any(|pair| matches!(pair[0].as_str(), "-i" | "--include") && pair[1].as_str() == "inplace")
}

fn sed_has_file_operand(tokens: &[String]) -> bool {
    let mut idx = 1;
    let mut saw_script = false;

    while idx < tokens.len() {
        let token = tokens[idx].as_str();

        if matches!(token, "--") {
            idx += 1;
            break;
        }

        if matches!(token, "-e" | "--expression" | "-f" | "--file") {
            idx += 2;
            saw_script = true;
            continue;
        }

        if token.starts_with("-e") || token.starts_with("--expression=") {
            idx += 1;
            saw_script = true;
            continue;
        }

        if token.starts_with("-f") || token.starts_with("--file=") {
            idx += 1;
            saw_script = true;
            continue;
        }

        if token.starts_with('-') {
            idx += 1;
            continue;
        }

        if saw_script {
            return true;
        }

        saw_script = true;
        idx += 1;
    }

    idx < tokens.len()
}

fn awk_has_file_operand(tokens: &[String]) -> bool {
    let mut idx = 1;
    let mut saw_program = false;

    while idx < tokens.len() {
        let token = tokens[idx].as_str();

        if matches!(token, "--") {
            idx += 1;
            break;
        }

        if matches!(
            token,
            "-f" | "--file" | "-v" | "--assign" | "-F" | "--field-separator"
        ) {
            idx += 2;
            if matches!(token, "-f" | "--file") {
                saw_program = true;
            }
            continue;
        }

        if token.starts_with("-f")
            || token.starts_with("--file=")
            || token.starts_with("-v")
            || token.starts_with("--assign=")
            || token.starts_with("-F")
            || token.starts_with("--field-separator=")
        {
            if token.starts_with("-f") || token.starts_with("--file=") {
                saw_program = true;
            }
            idx += 1;
            continue;
        }

        if token.starts_with('-') {
            idx += 1;
            continue;
        }

        if saw_program {
            return true;
        }

        saw_program = true;
        idx += 1;
    }

    idx < tokens.len()
}

fn head_or_tail_has_file_operand(tokens: &[String]) -> bool {
    let mut idx = 1;

    while idx < tokens.len() {
        let token = tokens[idx].as_str();
        if token == "--" {
            return idx + 1 < tokens.len();
        }

        if matches!(
            token,
            "-n" | "--lines" | "-c" | "--bytes" | "-s" | "--sleep-interval" | "--pid"
        ) {
            idx += 2;
            continue;
        }

        if token.starts_with('-') {
            idx += 1;
            continue;
        }

        return true;
    }

    false
}

fn grep_has_file_operand(tokens: &[String]) -> bool {
    if tokens
        .iter()
        .skip(1)
        .any(|token| matches!(token.as_str(), "-f" | "--file") || token.starts_with("--file="))
    {
        return true;
    }

    let positional_count = tokens
        .iter()
        .skip(1)
        .filter(|token| !token.starts_with('-'))
        .count();
    positional_count >= 2
}

fn classify_file_operation_segment(segment: &str) -> Option<BashFileOperation> {
    let tokens = tokenize_shell(segment);
    let tokens = strip_invocation_wrappers(&tokens);
    let command = command_basename(tokens.first()?);

    let (kind, recommended_tool) = match command.as_str() {
        "head" | "tail" if !head_or_tail_has_file_operand(tokens) => return None,
        "cat" if has_unquoted_redirect(segment) => (BashFileOperationKind::Edit, "write"),
        "cat" | "head" | "tail" | "less" | "more" => (BashFileOperationKind::Read, "read"),
        "grep" if !grep_has_file_operand(tokens) => return None,
        "grep" | "rg" => (BashFileOperationKind::Search, "grep"),
        "find" => (BashFileOperationKind::Search, "glob/list"),
        "sed" if sed_in_place(tokens) || has_unquoted_redirect(segment) => {
            (BashFileOperationKind::Edit, "edit")
        }
        "sed" if sed_has_file_operand(tokens) => (BashFileOperationKind::Read, "read"),
        "awk" if awk_in_place(tokens) || has_unquoted_redirect(segment) => {
            (BashFileOperationKind::Edit, "edit")
        }
        "awk" if awk_has_file_operand(tokens) => (BashFileOperationKind::Read, "read"),
        _ => return None,
    };

    Some(BashFileOperation {
        kind,
        command,
        segment: segment.to_string(),
        recommended_tool,
    })
}

fn command_basename(command: &str) -> String {
    std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_ascii_lowercase()
}

pub(crate) fn split_shell_segments(command: &str) -> Vec<String> {
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

fn contains_shell_glob(suffix: &str) -> bool {
    suffix.chars().any(|ch| matches!(ch, '*' | '?' | '[' | '{'))
}

fn is_dangerous_home_rm_target(target: &str) -> bool {
    matches!(
        target,
        "~" | "~/" | "$HOME" | "$HOME/" | "${HOME}" | "${HOME}/"
    ) || target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("$HOME/"))
        .or_else(|| target.strip_prefix("${HOME}/"))
        .is_some_and(contains_shell_glob)
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
            matches!(target.as_str(), "/" | "/*")
                || is_dangerous_home_rm_target(target)
                || target.starts_with("/etc")
                || target.starts_with("/usr")
                || target.starts_with("/var")
        })
}

fn dangerous_git_reason(tokens: &[String]) -> Option<&'static str> {
    if command_basename(tokens.first()?) != "git" {
        return None;
    }

    let mut idx = 1;
    while idx < tokens.len() {
        let token = tokens[idx].as_str();
        if token == "--" {
            return None;
        }
        if token == "-C" || token == "-c" || token == "--git-dir" || token == "--work-tree" {
            idx = (idx + 2).min(tokens.len());
            continue;
        }
        if token.starts_with("-C") && token.len() > 2 {
            idx += 1;
            continue;
        }
        if token.starts_with("-c") && token.len() > 2 {
            idx += 1;
            continue;
        }
        if token.starts_with("--git-dir=") || token.starts_with("--work-tree=") {
            idx += 1;
            continue;
        }
        if token.starts_with('-') {
            idx += 1;
            continue;
        }
        break;
    }

    let subcommand = tokens.get(idx).map(|token| token.as_str())?;
    let args = &tokens[idx + 1..];

    match subcommand {
        "reset" if args.iter().any(|arg| arg == "--hard") => Some("destructive git reset --hard"),
        "push"
            if args
                .iter()
                .any(|arg| *arg == "-f" || arg.starts_with("--force")) =>
        {
            Some("destructive git force push")
        }
        "checkout" if git_checkout_discards_paths(args) => {
            Some("destructive git checkout path restore")
        }
        "restore" if git_restore_discards_paths(args) => Some("destructive git restore"),
        "clean" if git_clean_deletes_files(args) => Some("destructive git clean"),
        "branch" if args.iter().any(|arg| *arg == "-D") => Some("destructive git branch delete"),
        _ => None,
    }
}

fn git_checkout_discards_paths(args: &[String]) -> bool {
    let mut saw_separator = false;
    let mut non_option_count = 0;

    for arg in args {
        if saw_separator {
            return true;
        }
        if arg == "--" {
            saw_separator = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        non_option_count += 1;
    }

    non_option_count >= 2
}

fn git_restore_discards_paths(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--staged")
        || args.iter().any(|arg| !arg.starts_with('-') && arg != "--")
}

fn git_clean_deletes_files(args: &[String]) -> bool {
    let has_dry_run = args.iter().any(|arg| {
        arg == "-n"
            || arg == "--dry-run"
            || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('n'))
    });
    if has_dry_run {
        return false;
    }

    args.iter().any(|arg| {
        arg == "-f"
            || arg == "--force"
            || arg.starts_with("--force=")
            || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('f'))
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
    let tokens = strip_invocation_wrappers(&tokens);
    let command = tokens.first().map(|t| command_basename(t))?;

    if matches!(command.as_str(), "sudo" | "doas" | "su") {
        return Some("privilege escalation");
    }

    if command == "rm" && is_dangerous_rm(tokens) {
        return Some("destructive rm target");
    }

    if let Some(reason) = dangerous_git_reason(tokens) {
        return Some(reason);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(command: &str) -> Option<(BashFileOperationKind, String, &'static str)> {
        classify_bash_command(command)
            .file_operation
            .map(|operation| {
                (
                    operation.kind,
                    operation.command,
                    operation.recommended_tool,
                )
            })
    }

    #[test]
    fn classifies_file_read_commands() {
        assert_eq!(
            classify("cat /tmp/file.txt"),
            Some((BashFileOperationKind::Read, "cat".to_string(), "read"))
        );
        assert_eq!(
            classify("DEBUG=1 head -n 20 src/lib.rs"),
            Some((BashFileOperationKind::Read, "head".to_string(), "read"))
        );
        assert_eq!(
            classify("timeout 5 tail -n 50 /var/log/app.log"),
            Some((BashFileOperationKind::Read, "tail".to_string(), "read"))
        );
        assert_eq!(
            classify("/usr/bin/cat ./Cargo.toml"),
            Some((BashFileOperationKind::Read, "cat".to_string(), "read"))
        );
    }

    #[test]
    fn classifies_cat_redirects_as_file_edits() {
        assert_eq!(
            classify("cat > src/generated.rs <<'EOF'\nfn main() {}\nEOF"),
            Some((BashFileOperationKind::Edit, "cat".to_string(), "write"))
        );
    }

    #[test]
    fn allows_head_and_tail_when_they_only_bound_piped_output() {
        assert_eq!(classify("ls -la | head -100"), None);
        assert_eq!(classify("cargo test | tail -n 20"), None);
        assert_eq!(classify("printf 'one\\ntwo\\n' | head -n 1"), None);
    }

    #[test]
    fn classifies_file_search_commands() {
        assert_eq!(
            classify("grep -R \"needle\" src"),
            Some((BashFileOperationKind::Search, "grep".to_string(), "grep"))
        );
        assert_eq!(
            classify("env RUST_LOG=debug rg needle crates"),
            Some((BashFileOperationKind::Search, "rg".to_string(), "grep"))
        );
        assert_eq!(
            classify("find . -name '*.rs'"),
            Some((
                BashFileOperationKind::Search,
                "find".to_string(),
                "glob/list"
            ))
        );
    }

    #[test]
    fn allows_grep_when_it_only_filters_standard_input() {
        assert_eq!(classify("ss -ltnp | grep 5291"), None);
        assert_eq!(classify("printf 'one\\ntwo\\n' | grep two"), None);
        assert_eq!(
            classify("grep needle src/lib.rs"),
            Some((BashFileOperationKind::Search, "grep".to_string(), "grep"))
        );
    }

    #[test]
    fn classifies_file_edit_commands() {
        assert_eq!(
            classify("sed -i 's/old/new/' src/lib.rs"),
            Some((BashFileOperationKind::Edit, "sed".to_string(), "edit"))
        );
        assert_eq!(
            classify("awk -i inplace '{print}' src/lib.rs"),
            Some((BashFileOperationKind::Edit, "awk".to_string(), "edit"))
        );
        assert_eq!(
            classify("sed 's/old/new/' src/lib.rs > /tmp/out"),
            Some((BashFileOperationKind::Edit, "sed".to_string(), "edit"))
        );
    }

    #[test]
    fn classifies_non_mutating_sed_and_awk_file_operands_as_reads() {
        assert_eq!(
            classify("sed -n '1,20p' src/lib.rs"),
            Some((BashFileOperationKind::Read, "sed".to_string(), "read"))
        );
        assert_eq!(
            classify("awk '{print $1}' src/lib.rs"),
            Some((BashFileOperationKind::Read, "awk".to_string(), "read"))
        );
    }

    #[test]
    fn ignores_non_file_operation_commands() {
        assert_eq!(classify("cargo test -p krusty-core"), None);
        assert_eq!(classify("git status --short"), None);
        assert_eq!(classify("echo 'cat file'"), None);
    }

    #[test]
    fn classifies_first_file_operation_across_segments() {
        assert_eq!(
            classify("cargo check && rg needle crates"),
            Some((BashFileOperationKind::Search, "rg".to_string(), "grep"))
        );
    }

    #[test]
    fn returns_combined_classification() {
        let classification =
            classify_bash_command("git reset --hard && sed -i 's/a/b/' src/lib.rs");
        assert_eq!(
            classification.safety_violation.as_deref(),
            Some("destructive git reset --hard")
        );
        assert!(matches!(
            classification
                .file_operation
                .map(|operation| operation.kind),
            Some(BashFileOperationKind::Edit)
        ));
        assert!(classification.modifies_filesystem_or_process);
    }

    #[test]
    fn observation_signature_preserves_ls_resource_operand() {
        assert_ne!(
            semantic_bash_signature("ls -la src"),
            semantic_bash_signature("ls -la other")
        );
    }

    #[test]
    fn unknown_and_interpreter_shell_surfaces_are_not_treated_as_observations() {
        for command in [
            "python3 -c 'open(\"out\", \"w\").write(\"x\")'",
            "node -e 'require(\"fs\").writeFileSync(\"out\", \"x\")'",
            "sh -c 'touch out'",
            "custom-build-driver --output out",
            "uniq input.txt output.txt",
            "file --compile -m payload.magic",
            "sort ${KRUSTY_UNSET:--o} output.txt input.txt",
            "command env PATH=./attacker-bin ls",
            "ECHO harmless-looking",
            "echo \"$RANDOM\"",
        ] {
            assert!(
                classify_bash_command(command).modifies_filesystem_or_process,
                "unproven command must be effectful: {command}"
            );
        }

        for command in ["pwd", "git status --short", "cd . && rg needle src"] {
            assert!(
                !classify_bash_command(command).modifies_filesystem_or_process,
                "audited command should remain observational: {command}"
            );
        }
    }

    #[test]
    fn treats_general_purpose_interpreters_as_write_capable() {
        for command in [
            "python3 - <<'PY'\nfrom pathlib import Path\nPath('server.py').write_text('ok')\nPY",
            "python3.14 -c \"from pathlib import Path; Path('server.py').write_text('ok')\"",
            "node -e \"require('fs').writeFileSync('server.js', 'ok')\"",
            "sh -c 'touch generated.txt'",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "expected interpreter command to be treated as write-capable: {command}"
            );
        }

        assert!(is_write_capable_in_plan_mode("mkdir generated"));
        assert!(is_write_capable_in_plan_mode("git status --short"));
        assert!(is_write_capable_in_plan_mode("cat Cargo.toml"));
    }

    #[test]
    fn plan_mode_denies_unproven_shell_surfaces_and_expansion_bypasses() {
        for command in [
            "dash -c 'touch generated.txt'",
            "php -r 'file_put_contents(\"generated.txt\", \"x\");'",
            "lua -e 'io.open(\"generated.txt\", \"w\")'",
            "deno eval 'Deno.writeTextFileSync(\"generated.txt\", \"x\")'",
            "awk 'BEGIN { system(\"touch generated.txt\") }'",
            "find . -exec touch generated.txt ;",
            "curl -o generated.txt https://example.com/file",
            "echo $(touch generated.txt)",
            "cat <(touch generated.txt)",
            "PAGER='touch generated.txt' git log",
            "git fetch origin",
            "./git status",
            "echo ok\ntouch generated.txt",
            "printf '%s' ok\nrm -f victim.txt",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "expected unproven Plan-mode command to be blocked: {command}"
            );
        }
    }

    #[test]
    fn plan_mode_rejects_write_capable_modes_of_observational_commands() {
        for command in [
            "sort -o out.txt input.txt",
            "sort --output=out.txt input.txt",
            "tree -o tree.txt .",
            "tree --output=tree.txt .",
            "git diff --output=patch.diff",
            "git diff --output patch.diff",
            "git diff --ext-diff",
            "git grep --open-files-in-pager needle",
            "git grep --open-files-in-pager='sh -c touch-out' needle",
            "git show --show-signature HEAD",
            "rg --pre 'sh -c touch-out' needle .",
            "rg --pre='sh -c touch-out' needle .",
            "rg --hostname-bin 'sh -c touch-out' needle .",
            "date --set='2026-07-22'",
            "date -s 2026-07-22",
            "date 072212002026",
            "hostname new-name",
            "ss -K dst 127.0.0.1",
            "printf -v value '%s' payload",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "write-capable command mode must not be Plan-safe: {command}"
            );
            assert!(
                classify_bash_command(command).modifies_filesystem_or_process,
                "write-capable command mode must be effectful: {command}"
            );
        }
    }

    #[test]
    fn plan_mode_blocks_even_observational_shell_forms() {
        for command in [
            "sort input.txt",
            "tree .",
            "git diff --stat",
            "git grep needle",
            "rg needle crates",
            "date -u +%F",
            "hostname",
            "ss -ltnp",
            "printf '%s' payload",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "free-form shell must not become Plan-safe: {command}"
            );
        }
    }

    #[test]
    fn plan_mode_blocks_the_former_read_only_shell_allowlist() {
        for command in [
            "git status --short",
            "git diff --stat",
            "git branch --show-current",
            "rg needle crates | head -20",
            "cat Cargo.toml",
            "ps -ef | grep krusty",
            "sha256sum target/release/krusty",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "free-form shell must be blocked in Plan mode: {command}"
            );
        }
    }

    #[test]
    fn wrapped_mutating_commands_remain_write_capable_in_plan_mode() {
        for command in [
            "command mkdir generated",
            "env mkdir generated",
            "timeout 1 mkdir generated",
        ] {
            assert!(
                is_write_capable_in_plan_mode(command),
                "expected wrapped mutating command to be write-capable: {command}"
            );
        }
    }

    #[test]
    fn classifies_allowed_build_commands_as_non_file_operations() {
        for command in [
            "cargo check --workspace",
            "cargo test -p krusty-core",
            "npm run build",
            "make test",
        ] {
            let classification = classify_bash_command(command);
            assert_eq!(
                classification.safety_violation, None,
                "expected {command:?} to have no safety violation"
            );
            assert_eq!(
                classification.file_operation, None,
                "expected {command:?} not to be classified as file-operation misuse"
            );
        }
    }

    #[test]
    fn detects_destructive_git_commands() {
        for (command, reason) in [
            ("git reset --hard HEAD~1", "destructive git reset --hard"),
            (
                "git push --force-with-lease origin main",
                "destructive git force push",
            ),
            (
                "git checkout -- src/lib.rs",
                "destructive git checkout path restore",
            ),
            ("git restore src/lib.rs", "destructive git restore"),
            ("git clean -fd", "destructive git clean"),
            (
                "git branch -D stale-branch",
                "destructive git branch delete",
            ),
            (
                "env GIT_OPTIONAL_LOCKS=0 git -C /tmp/repo reset --hard",
                "destructive git reset --hard",
            ),
        ] {
            assert_eq!(
                classify_bash_command(command).safety_violation,
                Some(reason.to_string()),
                "expected {command:?} to be classified as destructive"
            );
        }
    }

    #[test]
    fn allows_non_destructive_git_commands() {
        for command in [
            "git status --short",
            "git diff -- src/lib.rs",
            "git checkout feature-branch",
            "git clean -nfd",
            "git branch -d topic",
        ] {
            assert_eq!(
                classify_bash_command(command).safety_violation,
                None,
                "expected {command:?} to remain allowed by safety classification"
            );
        }
    }
}
