use std::path::Path;

use tracing::warn;

use crate::paths;
use crate::storage::{
    MakoCrewProfileDocumentKind, MakoHomeProfile, MakoProfileDocumentKind, MakoProfileSnapshot,
};

use super::project::discover_named_file;
use super::truncate_utf8_bytes;

const MAKO_FILES: &[&str] = &["HIVE.md", "hive.md", "MAKO.md", "mako.md"];
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
    let layers = profile.context_layers();
    let mut sections = layers
        .iter()
        .filter(|layer| matches!(layer.kind, "SOUL" | "IDENTITY" | "USER"))
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

    // Operational guidance follows every stable persona layer. Legacy memory
    // (including crew MEMORY.md) is deliberately never an active instruction;
    // continuity comes from exact-owner canonical memory and episodes.
    sections.extend(
        layers
            .iter()
            .filter(|layer| matches!(layer.kind, "HEARTBEAT" | "CHANNELS"))
            .map(|layer| {
                format!(
                    "[MAKO {} - {}]\n\n{}\n\n[END MAKO {}]",
                    layer.kind, layer.document.file_name, layer.document.content, layer.kind
                )
            }),
    );

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Hive project overlay") {
            let label = display_context_file_name(&path, "HIVE.md");
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
    for kind in [
        MakoProfileDocumentKind::Soul,
        MakoProfileDocumentKind::Identity,
        MakoProfileDocumentKind::User,
    ] {
        if let Some(document) = profile.document(kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[MAKO {label} - profile:{}]\n\n{}\n\n[END MAKO {label}]",
                profile.profile_id, document.content
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
                        "[MAKO CREW {label} - {} - crew-revision:{}]\n\n{}\n\n[END MAKO CREW {label}]",
                        member.slug, member.revision, document.content
                    ));
                }
            }
        }
    }

    for kind in [
        MakoProfileDocumentKind::Heartbeat,
        MakoProfileDocumentKind::Channels,
    ] {
        if let Some(document) = profile.document(kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[MAKO {label} - profile:{} - snapshot-revision:{}]\n\n{}\n\n[END MAKO {label}]",
                profile.profile_id, profile.revision, document.content
            ));
        }
    }

    if let Some(path) = discover_named_file(project_root, MAKO_FILES) {
        if let Some(content) = load_mako_context_file(&path, "Hive project overlay") {
            let label = display_context_file_name(&path, "HIVE.md");
            sections.push(format!(
                "[MAKO PROJECT OVERLAY - {}]\n\n{}\n\n[END MAKO PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES)
}

fn bound_mako_sections(sections: Vec<String>, max_bytes: usize) -> Vec<String> {
    if max_bytes == 0 || sections.is_empty() {
        return Vec::new();
    }

    const MARKER: &str = "\n\n[MAKO CONTEXT TRUNCATED AT REQUEST BUDGET]";
    // Reserve separators pessimistically so the returned joined payload can
    // never exceed the byte ceiling even when every section is represented.
    let mut remaining = max_bytes.saturating_sub(sections.len().saturating_sub(1) * 2);
    let mut retained = vec![None; sections.len()];

    for priority in [3_u8, 2, 1] {
        let indexes = sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (mako_context_priority(section) == priority).then_some(index)
            })
            .collect::<Vec<_>>();
        if indexes.is_empty() {
            continue;
        }

        let required = indexes
            .iter()
            .map(|index| sections[*index].len())
            .sum::<usize>();
        if required <= remaining {
            for index in indexes {
                remaining -= sections[index].len();
                retained[index] = Some(sections[index].clone());
            }
            continue;
        }

        // The current tier consumes the rest of the budget. Divide it across
        // the tier so one oversized SOUL cannot erase IDENTITY or USER, and no
        // lower-priority heartbeat/project material can displace persona.
        let mut indexes_left = indexes.len();
        for index in indexes {
            let share = remaining / indexes_left;
            let section = &sections[index];
            let bounded = if section.len() <= share {
                section.clone()
            } else if share <= MARKER.len() {
                truncate_utf8_bytes(MARKER, share)
            } else {
                let mut value = truncate_utf8_bytes(section, share - MARKER.len());
                value.push_str(MARKER);
                value
            };
            remaining = remaining.saturating_sub(bounded.len());
            retained[index] = (!bounded.is_empty()).then_some(bounded);
            indexes_left -= 1;
        }
        break;
    }

    sections
        .into_iter()
        .enumerate()
        .filter_map(|(index, _)| retained[index].take())
        .collect()
}

fn mako_context_priority(section: &str) -> u8 {
    if [
        "[MAKO SOUL",
        "[MAKO IDENTITY",
        "[MAKO USER",
        "[MAKO CREW IDENTITY",
        "[MAKO CREW SOUL",
    ]
    .iter()
    .any(|prefix| section.starts_with(prefix))
    {
        3
    } else if section.starts_with("[MAKO HEARTBEAT") || section.starts_with("[MAKO CHANNELS") {
        2
    } else {
        1
    }
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
    use tempfile::TempDir;

    use super::{
        bound_mako_sections, build_mako_context_sections_with_profile, MAX_MAKO_CONTEXT_BYTES,
    };
    use crate::storage::{MakoProfileDocument, MakoProfileDocumentKind, MakoProfileSnapshot};

    fn document(
        kind: MakoProfileDocumentKind,
        content: &str,
    ) -> MakoProfileDocument<MakoProfileDocumentKind> {
        MakoProfileDocument {
            kind,
            content: content.to_string(),
            updated_at: "2026-07-17T00:00:00Z".to_string(),
        }
    }

    fn profile(revision: i64, heartbeat: &str, channels: &str) -> MakoProfileSnapshot {
        MakoProfileSnapshot {
            profile_id: "local".to_string(),
            user_id: None,
            revision,
            soul: Some(document(MakoProfileDocumentKind::Soul, "Warm and candid.")),
            identity: Some(document(MakoProfileDocumentKind::Identity, "Name: Mako")),
            user: Some(document(
                MakoProfileDocumentKind::User,
                "Prefer concise progress updates.",
            )),
            heartbeat: Some(document(MakoProfileDocumentKind::Heartbeat, heartbeat)),
            channels: Some(document(MakoProfileDocumentKind::Channels, channels)),
            crew: Vec::new(),
        }
    }

    #[test]
    fn mako_sections_respect_aggregate_request_budget() {
        let sections = vec!["a".repeat(20_000), "🦈".repeat(10_000)];
        let bounded = bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES);
        let request_bytes =
            bounded.iter().map(String::len).sum::<usize>() + bounded.len().saturating_sub(1) * 2;

        assert!(request_bytes <= MAX_MAKO_CONTEXT_BYTES);
        assert!(bounded.join("\n\n").contains("TRUNCATED"));
    }

    #[test]
    fn mako_section_budget_preserves_every_stable_identity_before_volatile_context() {
        let sections = vec![
            format!("[MAKO SOUL - profile:local]\n{}", "s".repeat(20_000)),
            format!("[MAKO IDENTITY - profile:local]\n{}", "i".repeat(20_000)),
            format!("[MAKO USER - profile:local]\n{}", "u".repeat(20_000)),
            format!("[MAKO HEARTBEAT - profile:local]\n{}", "h".repeat(20_000)),
            format!("[MAKO PROJECT OVERLAY]\n{}", "p".repeat(20_000)),
        ];

        let bounded = bound_mako_sections(sections, MAX_MAKO_CONTEXT_BYTES);
        let joined = bounded.join("\n\n");

        assert!(joined.contains("[MAKO SOUL"));
        assert!(joined.contains("[MAKO IDENTITY"));
        assert!(joined.contains("[MAKO USER"));
        assert!(!joined.contains("[MAKO HEARTBEAT"));
        assert!(!joined.contains("[MAKO PROJECT OVERLAY"));
        assert!(joined.len() <= MAX_MAKO_CONTEXT_BYTES);
    }

    #[test]
    fn volatile_profile_edits_do_not_change_rendered_stable_identity_prefix() {
        let project = TempDir::new().unwrap();
        let first = build_mako_context_sections_with_profile(
            project.path(),
            &profile(4, "Check queue A.", "Main thread."),
            None,
        );
        let second = build_mako_context_sections_with_profile(
            project.path(),
            &profile(5, "Check queue B.", "Mobile push."),
            None,
        );
        let stable = |sections: &[String]| {
            sections
                .iter()
                .filter(|section| {
                    section.starts_with("[MAKO SOUL")
                        || section.starts_with("[MAKO IDENTITY")
                        || section.starts_with("[MAKO USER")
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let volatile = |sections: &[String]| {
            sections
                .iter()
                .filter(|section| {
                    section.starts_with("[MAKO HEARTBEAT") || section.starts_with("[MAKO CHANNELS")
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        assert_eq!(stable(&first), stable(&second));
        assert_ne!(volatile(&first), volatile(&second));
    }
}
