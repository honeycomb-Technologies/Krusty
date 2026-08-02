/// Generate KRAB.md template content.
pub(super) fn generate_krab_template(
    project_name: &str,
    languages: &[String],
    structure: &[(String, String)],
) -> String {
    let mut content = String::new();

    content.push_str(&format!("# {}\n\n", project_name));
    content.push_str("<!-- KRAB.md - Project context for Mitsuro AI assistant -->\n");
    content.push_str(
        "<!-- This file is automatically read at the start of every AI conversation -->\n",
    );
    content.push_str(
        "<!-- Edit it to help the AI understand your project's rules and conventions -->\n\n",
    );

    content.push_str("## Overview\n\n");
    content.push_str("<!-- Describe what this project does and its main purpose -->\n\n");
    content.push_str("TODO: Add project description\n\n");

    content.push_str("## Tech Stack\n\n");
    for lang in languages {
        content.push_str(&format!("- {}\n", lang));
    }
    content.push('\n');

    if !structure.is_empty() {
        content.push_str("## Directory Structure\n\n");
        for (dir, desc) in structure {
            content.push_str(&format!("- `{}` - {}\n", dir, desc));
        }
        content.push('\n');
    }

    content.push_str("## Key Files\n\n");
    content.push_str("<!-- List important files the AI should know about -->\n\n");
    content.push_str("- `KRAB.md` - This file (project context)\n");
    content.push('\n');

    content.push_str("## Conventions\n\n");
    content.push_str("<!-- Describe coding style, naming conventions, etc. -->\n\n");
    content.push_str("TODO: Add coding conventions\n\n");

    content.push_str("## Build & Run\n\n");
    content.push_str("<!-- How to build, run, and test the project -->\n\n");
    content.push_str("```bash\n");
    content.push_str("# TODO: Add build commands\n");
    content.push_str("```\n\n");

    content.push_str("## Notes for AI\n\n");
    content.push_str("<!-- Any specific instructions or context for the AI assistant -->\n\n");
    content.push_str("TODO: Add any project-specific notes\n");

    content
}

/// Clean AI output - remove filler phrases and meta-commentary.
fn clean_ai_output(text: &str) -> String {
    const FILLER_STARTS: &[&str] = &[
        "Perfect!",
        "Great!",
        "Excellent!",
        "Now I",
        "Let me",
        "I will",
        "I'll",
        "Based on my",
        "After analyzing",
        "Here's what I found",
        "Here is",
        "Summary:",
        "## Summary",
        "### Summary",
        "Analysis:",
        "## Analysis",
    ];

    let mut lines: Vec<&str> = text.lines().collect();

    const NOISE_SECTIONS: &[&str] = &["### Files Examined", "### Sources"];
    let mut in_noise_section = false;
    lines.retain(|line| {
        let trimmed = line.trim();
        if NOISE_SECTIONS.iter().any(|s| trimmed.starts_with(s)) {
            in_noise_section = true;
            return false;
        }
        if in_noise_section {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                in_noise_section = false;
            } else {
                return false;
            }
        }
        if trimmed.starts_with("## ") {
            return false;
        }
        !FILLER_STARTS.iter().any(|f| trimmed.starts_with(f))
    });

    let mut result = String::new();
    let mut blank_count = 0;
    for line in lines {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Generate KRAB.md from AI exploration results.
pub fn generate_krab_from_exploration(
    project_name: &str,
    languages: &[String],
    exploration: &crate::tui::utils::InitExplorationResult,
) -> String {
    let mut content = String::new();

    content.push_str(&format!("# {}\n\n", project_name));

    content.push_str("## Tech Stack\n\n");
    for lang in languages {
        content.push_str(&format!("- {}\n", lang));
    }
    content.push('\n');

    let arch = clean_ai_output(&exploration.architecture);
    if !arch.is_empty() {
        content.push_str("## Architecture\n\n");
        content.push_str(&arch);
        content.push_str("\n\n");
    }

    let files = clean_ai_output(&exploration.key_files);
    if !files.is_empty() {
        content.push_str("## Key Files\n\n");
        content.push_str(&files);
        content.push_str("\n\n");
    }

    let conv = clean_ai_output(&exploration.conventions);
    if !conv.is_empty() {
        content.push_str("## Conventions\n\n");
        content.push_str(&conv);
        content.push_str("\n\n");
    }

    let build = clean_ai_output(&exploration.build_system);
    if !build.is_empty() {
        content.push_str("## Build & Run\n\n");
        content.push_str(&build);
        content.push_str("\n\n");
    }

    content.push_str("## Notes for AI\n\n");
    content.push_str("<!-- Add project-specific instructions here -->\n\n");

    content
}
