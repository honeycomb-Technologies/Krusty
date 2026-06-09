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
    let modifies_filesystem_or_process = segments
        .iter()
        .any(|segment| is_mutating_shell_segment(segment));

    BashCommandClassification {
        safety_violation,
        file_operation,
        modifies_filesystem_or_process,
    }
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

fn classify_file_operation_segment(segment: &str) -> Option<BashFileOperation> {
    let tokens = tokenize_shell(segment);
    let tokens = strip_invocation_wrappers(&tokens);
    let command = command_basename(tokens.first()?);

    let (kind, recommended_tool) = match command.as_str() {
        "cat" | "head" | "tail" | "less" | "more" => (BashFileOperationKind::Read, "read"),
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
