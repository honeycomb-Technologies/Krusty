use std::path::Path;

use tracing::warn;

use crate::paths;
use crate::storage::{
    MakoCrewProfileDocumentKind, MakoProfileDocumentKind, MakoProfileSnapshot, MakoHomeProfile,
};

use super::project::discover_named_file;
use super::truncate_utf8_bytes;

const MAKO_FILES: &[&str] = &["MAKO.md", "mako.md"];
const MAX_MAKO_CONTEXT_BYTES: usize = 24 * 1024;

pub(super) fn build_mako_context_sections(
    project_root: &Path,
    mako_crew_slug: Option<&str>,
) -> Vec<String> {
    let mako_home = paths::mako_dir();
    build_mako_context_sections_with_home(project_root, &mako_home, mako_crew_slug)
}

pub(super) fn build_mako_context_sections_with_home(
    project_root: &Path,
    mako_home: &Path,
    mako_crew_slug: Option<&str>,
) -> Vec<String> {
    let profile = MakoHomeProfile::load_from(mako_home);
    let mut sections = profile
        .context_layers()
        .into_iter()
        .map(|layer| {
            format!(
                "[MAKO {} - {}]\n\n{}\n\n[END MAKO {}]",
                layer.kind, layer.document.file_name, layer.document.content, layer.kind
            )
        })
        .collect::<Vec<_>>();

    if let Some(crew_slug) = mako_crew_slug
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(member) = profile.crew.iter().find(|member| member.slug == crew_slug) {
            for (kind, document) in [
                ("CREW IDENTITY", member.identity.as_ref()),
                ("CREW SOUL", member.soul.as_ref()),
                ("CREW MEMORY", member.memory.as_ref()),
            ] {
                if let Some(document) = document {
                    sections.push(format!(
                        "[MAKO {} - {} - {}]\n\n{}\n\n[END MAKO {}]",
                        kind, member.slug, document.file_name, document.content, kind
                    ));
                }
            }
        }
    }

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Mako project overlay") {
            let label = display_context_file_name(&path, "MAKO.md");
            sections.push(format!(
                "[MAKO PROJECT OVERLAY - {}]\n\n{}\n\n[END MAKO PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES)
}

/// Render a database-owned profile snapshot that was frozen at run start.
/// Soul, Identity, and User are stable identity layers; Heartbeat and Channels
/// remain dynamic operational context. Durable learned memory is injected from
/// the canonical memory store, never from the legacy home file.
pub(super) fn build_mako_context_sections_with_profile(
    project_root: &Path,
    profile: &MakoProfileSnapshot,
    mako_crew_slug: Option<&str>,
) -> Vec<String> {
    let mut sections = Vec::new();
    for kind in MakoProfileDocumentKind::ALL {
        if let Some(document) = profile.document(kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[MAKO {label} - profile:{} - revision:{}]\n\n{}\n\n[END MAKO {label}]",
                profile.profile_id, profile.revision, document.content
            ));
        }
    }

    if let Some(crew_slug) = mako_crew_slug
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(member) = profile.crew_member(crew_slug) {
            for kind in MakoCrewProfileDocumentKind::ALL {
                let document = match kind {
                    MakoCrewProfileDocumentKind::Identity => member.identity.as_ref(),
                    MakoCrewProfileDocumentKind::Soul => member.soul.as_ref(),
                };
                if let Some(document) = document {
                    let label = kind.as_str().to_ascii_uppercase();
                    sections.push(format!(
                        "[MAKO CREW {label} - {} - profile-revision:{} - crew-revision:{}]\n\n{}\n\n[END MAKO CREW {label}]",
                        member.slug, profile.revision, member.revision, document.content
                    ));
                }
            }
        }
    }

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Mako project overlay") {
            let label = display_context_file_name(&path, "MAKO.md");
            sections.push(format!(
                "[MAKO PROJECT OVERLAY - {}]\n\n{}\n\n[END MAKO PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES)
}

fn bound_mako_sections(sections: Vec<String>, max_bytes: usize) -> Vec<String> {
    if max_bytes == 0 {
        return Vec::new();
    }

    const MARKER: &str = "\n\n[MAKO CONTEXT TRUNCATED AT REQUEST BUDGET]";
    let mut bounded = Vec::new();
    let mut used = 0usize;

    for section in sections {
        let separator_bytes = usize::from(!bounded.is_empty()) * 2;
        let remaining = max_bytes
            .saturating_sub(used)
            .saturating_sub(separator_bytes);
        if remaining == 0 {
            break;
        }
        if section.len() <= remaining {
            used = used.saturating_add(separator_bytes + section.len());
            bounded.push(section);
            continue;
        }

        let truncated = if remaining <= MARKER.len() {
            truncate_utf8_bytes(MARKER, remaining)
        } else {
            let mut value = truncate_utf8_bytes(&section, remaining - MARKER.len());
            value.push_str(MARKER);
            value
        };
        bounded.push(truncated);
        break;
    }

    bounded
}

fn load_mako_context_file(path: &Path, context: &'static str) -> Option<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Mako context file");
            return None;
        }
    };

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn display_context_file_name(path: &Path, fallback: &str) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{bound_mako_sections, MAX_MAKO_CONTEXT_BYTES};

    #[test]
    fn mako_sections_respect_aggregate_request_budget() {
        let sections = vec!["a".repeat(20_000), "🦈".repeat(10_000)];
        let bounded = bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES);
        let request_bytes =
            bounded.iter().map(String::len).sum::<usize>() + bounded.len().saturating_sub(1) * 2;

        assert!(request_bytes <= MAX_MAKO_CONTEXT_BYTES);
        assert!(bounded.join("\n\n").contains("TRUNCATED"));
    }
}
