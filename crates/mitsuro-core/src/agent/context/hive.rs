use std::path::Path;

use tracing::warn;

use crate::paths;
use crate::storage::hive_groups;
use crate::storage::{
    HiveCrewProfileDocumentKind, HiveGroupRunContext, HiveGroupSenderKind, HiveHomeProfile,
    HiveProfileDocumentKind, HiveProfileSnapshot, HiveWorker, HiveWorkerDocument,
    HiveWorkerDocumentKind, HiveWorkerStore,
};

use super::project::discover_named_file;
use super::{open_context_database, truncate_utf8_bytes};

const HIVE_FILES: &[&str] = &[
    "HIVE.md",
    "hive.md",
    crate::identity::legacy::HIVE_PROJECT_OVERLAY_FILE_NAME,
    crate::identity::legacy::HIVE_PROJECT_OVERLAY_FILE_NAME_LOWERCASE,
];
const MAX_HIVE_CONTEXT_BYTES: usize = 24 * 1024;
/// Independent budget for the [GROUP ROOM] block so a chatty room cannot
/// displace persona or project context.
const MAX_GROUP_ROOM_CONTEXT_BYTES: usize = 16 * 1024;
/// Per-message excerpt bound inside the room timeline.
const MAX_GROUP_ROOM_MESSAGE_BYTES: usize = 1200;

/// Persona material for a session that is a Hive Worker's private DM lane.
pub(super) struct HiveWorkerPersona {
    /// Memory namespace granted to this Worker (Shared + this namespace).
    pub(super) memory_namespace_id: String,
    /// Rendered `[HIVE WORKER ...]` sections replacing the crew treatment.
    pub(super) sections: Vec<String>,
}

/// Resolve the Worker whose DM lane is this session, if any. The session
/// itself is ownership-checked upstream; the owner comparison here keeps a
/// mis-bound row from leaking another owner's persona or memory namespace.
pub(super) fn load_worker_persona(
    db_path: &Path,
    session_id: &str,
    user_id: Option<&str>,
) -> Option<HiveWorkerPersona> {
    let db = open_context_database(db_path, "loading Hive worker persona")?;
    let store = HiveWorkerStore::new(db);
    let worker = match store.get_by_dm_session(session_id) {
        Ok(worker) => worker?,
        Err(error) => {
            warn!(session_id, error = %error, "Failed to resolve Hive worker for DM session");
            return None;
        }
    };
    if worker.user_id.as_deref() != user_id {
        warn!(
            session_id,
            worker_id = %worker.id,
            "Hive worker DM binding does not match the session owner; skipping persona"
        );
        return None;
    }
    let documents = match store.documents(&worker.id) {
        Ok(documents) => documents,
        Err(error) => {
            warn!(worker_id = %worker.id, error = %error, "Failed to load Hive worker documents");
            Vec::new()
        }
    };
    Some(HiveWorkerPersona {
        sections: build_worker_persona_sections(&worker, &documents),
        memory_namespace_id: worker.memory_namespace_id,
    })
}

/// Resolve a Worker's memory namespace for a group member run that may not
/// be executing on that Worker's DM session.
pub(super) fn load_worker_memory_namespace(
    db_path: &Path,
    worker_id: &str,
    user_id: Option<&str>,
) -> Option<String> {
    let db = open_context_database(db_path, "loading Hive worker memory namespace")?;
    let store = HiveWorkerStore::new(db);
    let worker = match store.get(worker_id) {
        Ok(worker) => worker?,
        Err(error) => {
            warn!(worker_id, error = %error, "Failed to resolve Hive worker for group memory scope");
            return None;
        }
    };
    if worker.user_id.as_deref() != user_id {
        return None;
    }
    Some(worker.memory_namespace_id)
}

fn build_worker_persona_sections(
    worker: &HiveWorker,
    documents: &[HiveWorkerDocument],
) -> Vec<String> {
    let mut sections = vec![format!(
        "[HIVE WORKER - {slug}]\n\nYou are {name} (@{slug}), a dedicated Hive Worker. This session is your private DM lane with the user; speak and act as this Worker.\n\n[END HIVE WORKER]",
        slug = worker.slug,
        name = worker.display_name,
    )];
    for kind in HiveWorkerDocumentKind::ALL {
        if let Some(document) = documents.iter().find(|document| document.kind == kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[HIVE WORKER {label} - {slug}]\n\n{content}\n\n[END HIVE WORKER {label}]",
                slug = worker.slug,
                content = document.content,
            ));
        }
    }
    sections
}

/// Render the [GROUP ROOM] block for one member run of a group turn: group
/// title, member roster, posting contract, and the last
/// `context_window_messages` room messages. Ordinary assistant output stays
/// private to the run; only `post_to_group` reaches the room.
pub(super) fn build_group_room_section(
    db_path: &Path,
    group_run: &HiveGroupRunContext,
) -> Option<String> {
    let db = open_context_database(db_path, "loading Hive group room context")?;
    let conn = db.conn();
    let group = match hive_groups::load_group(conn, &group_run.group_id) {
        Ok(group) => group?,
        Err(error) => {
            warn!(group_id = %group_run.group_id, error = %error, "Failed to load Hive group for room context");
            return None;
        }
    };
    let roster = match hive_groups::load_member_workers(conn, &group_run.group_id) {
        Ok(roster) => roster,
        Err(error) => {
            warn!(group_id = %group_run.group_id, error = %error, "Failed to load Hive group roster");
            Vec::new()
        }
    };
    let window = group_run.context_window_messages.max(1) as usize;
    let messages = match hive_groups::load_recent_messages(conn, &group_run.group_id, window) {
        Ok(messages) => messages,
        Err(error) => {
            warn!(group_id = %group_run.group_id, error = %error, "Failed to load Hive group messages");
            Vec::new()
        }
    };

    let display_for = |worker_id: Option<&str>| -> String {
        worker_id
            .and_then(|id| roster.iter().find(|worker| worker.id == id))
            .map(|worker| format!("@{}", worker.slug))
            .unwrap_or_else(|| "@former-member".to_string())
    };
    let self_label = roster
        .iter()
        .find(|worker| worker.id == group_run.worker_id)
        .map(|worker| format!("{} (@{})", worker.display_name, worker.slug))
        .unwrap_or_else(|| "a group member".to_string());

    let mut section = format!(
        "[GROUP ROOM - {title}]\n\nYou are {self_label}, working in the group room \"{title}\" with the user and the members below. This run was triggered by a room message.\n\nMembers:\n",
        title = group.title,
    );
    for worker in &roster {
        let provider = worker
            .model_key
            .as_ref()
            .map(|key| format!("{}", key.provider))
            .or_else(|| worker.model.clone())
            .unwrap_or_else(|| "default model".to_string());
        section.push_str(&format!(
            "- @{} ({}, {}){}\n",
            worker.slug,
            worker.display_name,
            provider,
            if worker.id == group_run.worker_id {
                " <- you"
            } else {
                ""
            }
        ));
    }
    section.push_str(&format!(
        "\nRoom rules:\n- Everything you say normally (assistant text, tool use, reasoning) stays PRIVATE to this run.\n- To speak in the room, call the post_to_group tool. You may post at most {} message(s) this run; make them count.\n- Mention members with @slug when you address them. Reply to a specific message by passing its message id.\n- If you have nothing useful to add, do not post.\n",
        group_run.max_member_messages_per_turn
    ));

    if messages.is_empty() {
        section.push_str("\nThe room has no messages yet.\n");
    } else {
        section.push_str("\nRecent room messages (oldest first):\n");
        for message in &messages {
            let sender = match message.sender_kind {
                HiveGroupSenderKind::User => "user".to_string(),
                HiveGroupSenderKind::System => "system".to_string(),
                HiveGroupSenderKind::Worker => display_for(message.sender_worker_id.as_deref()),
            };
            let reply = message
                .reply_to_message_id
                .as_deref()
                .map(|id| format!(" (reply to {id})"))
                .unwrap_or_default();
            section.push_str(&format!(
                "#{seq} [{id}] {sender}{reply}: {content}\n",
                seq = message.seq,
                id = message.id,
                content = truncate_utf8_bytes(&message.content, MAX_GROUP_ROOM_MESSAGE_BYTES),
            ));
        }
    }
    section.push_str("\n[END GROUP ROOM]");

    if section.len() > MAX_GROUP_ROOM_CONTEXT_BYTES {
        let marker = "\n[GROUP ROOM TRUNCATED AT REQUEST BUDGET]\n[END GROUP ROOM]";
        let mut bounded = truncate_utf8_bytes(
            &section,
            MAX_GROUP_ROOM_CONTEXT_BYTES.saturating_sub(marker.len()),
        );
        bounded.push_str(marker);
        return Some(bounded);
    }
    Some(section)
}

pub(super) fn build_hive_context_sections(
    project_root: &Path,
    hive_crew_slug: Option<&str>,
    worker_persona_sections: &[String],
) -> Vec<String> {
    let hive_home = paths::hive_dir();
    build_hive_context_sections_with_home(
        project_root,
        &hive_home,
        hive_crew_slug,
        worker_persona_sections,
    )
}

pub(super) fn build_hive_context_sections_with_home(
    project_root: &Path,
    hive_home: &Path,
    hive_crew_slug: Option<&str>,
    worker_persona_sections: &[String],
) -> Vec<String> {
    let profile = HiveHomeProfile::load_from(hive_home);
    let layers = profile.context_layers();
    let mut sections = layers
        .iter()
        .filter(|layer| matches!(layer.kind, "SOUL" | "IDENTITY" | "USER"))
        .map(|layer| {
            format!(
                "[HIVE {} - {}]\n\n{}\n\n[END HIVE {}]",
                layer.kind, layer.document.file_name, layer.document.content, layer.kind
            )
        })
        .collect::<Vec<_>>();

    if !worker_persona_sections.is_empty() {
        // A Worker-bound DM replaces the generic crew treatment with the
        // Worker's own persona documents.
        sections.extend_from_slice(worker_persona_sections);
    } else if let Some(crew_slug) = hive_crew_slug
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
                        "[HIVE {} - {} - {}]\n\n{}\n\n[END HIVE {}]",
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
                    "[HIVE {} - {}]\n\n{}\n\n[END HIVE {}]",
                    layer.kind, layer.document.file_name, layer.document.content, layer.kind
                )
            }),
    );

    if let Some(path) = discover_named_file(project_root, HIVE_FILES) {
        if let Some(content) = load_hive_context_file(&path, "Hive project overlay") {
            let label = display_context_file_name(&path, "HIVE.md");
            sections.push(format!(
                "[HIVE PROJECT OVERLAY - {}]\n\n{}\n\n[END HIVE PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    bound_hive_sections(sections, MAX_HIVE_CONTEXT_BYTES)
}

/// Render a database-owned profile snapshot that was frozen at run start.
/// Soul, Identity, and User are stable identity layers; Heartbeat and Channels
/// remain dynamic operational context. Durable learned memory is injected from
/// the canonical memory store, never from the legacy home file.
pub(super) fn build_hive_context_sections_with_profile(
    project_root: &Path,
    profile: &HiveProfileSnapshot,
    hive_crew_slug: Option<&str>,
    worker_persona_sections: &[String],
) -> Vec<String> {
    let mut sections = Vec::new();
    for kind in [
        HiveProfileDocumentKind::Soul,
        HiveProfileDocumentKind::Identity,
        HiveProfileDocumentKind::User,
    ] {
        if let Some(document) = profile.document(kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[HIVE {label} - profile:{}]\n\n{}\n\n[END HIVE {label}]",
                profile.profile_id, document.content
            ));
        }
    }

    if !worker_persona_sections.is_empty() {
        sections.extend_from_slice(worker_persona_sections);
    } else if let Some(crew_slug) = hive_crew_slug
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(member) = profile.crew_member(crew_slug) {
            for kind in HiveCrewProfileDocumentKind::ALL {
                let document = match kind {
                    HiveCrewProfileDocumentKind::Identity => member.identity.as_ref(),
                    HiveCrewProfileDocumentKind::Soul => member.soul.as_ref(),
                };
                if let Some(document) = document {
                    let label = kind.as_str().to_ascii_uppercase();
                    sections.push(format!(
                        "[HIVE CREW {label} - {} - crew-revision:{}]\n\n{}\n\n[END HIVE CREW {label}]",
                        member.slug, member.revision, document.content
                    ));
                }
            }
        }
    }

    for kind in [
        HiveProfileDocumentKind::Heartbeat,
        HiveProfileDocumentKind::Channels,
    ] {
        if let Some(document) = profile.document(kind) {
            let label = kind.as_str().to_ascii_uppercase();
            sections.push(format!(
                "[HIVE {label} - profile:{} - snapshot-revision:{}]\n\n{}\n\n[END HIVE {label}]",
                profile.profile_id, profile.revision, document.content
            ));
        }
    }

    if let Some(path) = discover_named_file(project_root, HIVE_FILES) {
        if let Some(content) = load_hive_context_file(&path, "Hive project overlay") {
            let label = display_context_file_name(&path, "HIVE.md");
            sections.push(format!(
                "[HIVE PROJECT OVERLAY - {}]\n\n{}\n\n[END HIVE PROJECT OVERLAY]",
                label, content
            ));
        }
    }

    bound_hive_sections(sections, MAX_HIVE_CONTEXT_BYTES)
}

fn bound_hive_sections(sections: Vec<String>, max_bytes: usize) -> Vec<String> {
    if max_bytes == 0 || sections.is_empty() {
        return Vec::new();
    }

    const MARKER: &str = "\n\n[HIVE CONTEXT TRUNCATED AT REQUEST BUDGET]";
    // Reserve separators pessimistically so the returned joined payload can
    // never exceed the byte ceiling even when every section is represented.
    let mut remaining = max_bytes.saturating_sub(sections.len().saturating_sub(1) * 2);
    let mut retained = vec![None; sections.len()];

    for priority in [3_u8, 2, 1] {
        let indexes = sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (hive_context_priority(section) == priority).then_some(index)
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

fn hive_context_priority(section: &str) -> u8 {
    if [
        "[HIVE SOUL",
        "[HIVE IDENTITY",
        "[HIVE USER",
        "[HIVE CREW IDENTITY",
        "[HIVE CREW SOUL",
        "[HIVE WORKER",
    ]
    .iter()
    .any(|prefix| section.starts_with(prefix))
    {
        3
    } else if section.starts_with("[HIVE HEARTBEAT") || section.starts_with("[HIVE CHANNELS") {
        2
    } else {
        1
    }
}

fn load_hive_context_file(path: &Path, context: &'static str) -> Option<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Hive context file");
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
        bound_hive_sections, build_hive_context_sections_with_profile, MAX_HIVE_CONTEXT_BYTES,
    };
    use crate::storage::{HiveProfileDocument, HiveProfileDocumentKind, HiveProfileSnapshot};

    fn document(
        kind: HiveProfileDocumentKind,
        content: &str,
    ) -> HiveProfileDocument<HiveProfileDocumentKind> {
        HiveProfileDocument {
            kind,
            content: content.to_string(),
            updated_at: "2026-07-17T00:00:00Z".to_string(),
        }
    }

    fn profile(revision: i64, heartbeat: &str, channels: &str) -> HiveProfileSnapshot {
        HiveProfileSnapshot {
            profile_id: "local".to_string(),
            user_id: None,
            revision,
            soul: Some(document(HiveProfileDocumentKind::Soul, "Warm and candid.")),
            identity: Some(document(HiveProfileDocumentKind::Identity, "Name: Hive")),
            user: Some(document(
                HiveProfileDocumentKind::User,
                "Prefer concise progress updates.",
            )),
            heartbeat: Some(document(HiveProfileDocumentKind::Heartbeat, heartbeat)),
            channels: Some(document(HiveProfileDocumentKind::Channels, channels)),
            crew: Vec::new(),
        }
    }

    #[test]
    fn hive_sections_respect_aggregate_request_budget() {
        let sections = vec!["a".repeat(20_000), "🦈".repeat(10_000)];
        let bounded = bound_hive_sections(sections, MAX_HIVE_CONTEXT_BYTES);
        let request_bytes =
            bounded.iter().map(String::len).sum::<usize>() + bounded.len().saturating_sub(1) * 2;

        assert!(request_bytes <= MAX_HIVE_CONTEXT_BYTES);
        assert!(bounded.join("\n\n").contains("TRUNCATED"));
    }

    #[test]
    fn hive_section_budget_preserves_every_stable_identity_before_volatile_context() {
        let sections = vec![
            format!("[HIVE SOUL - profile:local]\n{}", "s".repeat(20_000)),
            format!("[HIVE IDENTITY - profile:local]\n{}", "i".repeat(20_000)),
            format!("[HIVE USER - profile:local]\n{}", "u".repeat(20_000)),
            format!("[HIVE HEARTBEAT - profile:local]\n{}", "h".repeat(20_000)),
            format!("[HIVE PROJECT OVERLAY]\n{}", "p".repeat(20_000)),
        ];

        let bounded = bound_hive_sections(sections, MAX_HIVE_CONTEXT_BYTES);
        let joined = bounded.join("\n\n");

        assert!(joined.contains("[HIVE SOUL"));
        assert!(joined.contains("[HIVE IDENTITY"));
        assert!(joined.contains("[HIVE USER"));
        assert!(!joined.contains("[HIVE HEARTBEAT"));
        assert!(!joined.contains("[HIVE PROJECT OVERLAY"));
        assert!(joined.len() <= MAX_HIVE_CONTEXT_BYTES);
    }

    #[test]
    fn volatile_profile_edits_do_not_change_rendered_stable_identity_prefix() {
        let project = TempDir::new().unwrap();
        let first = build_hive_context_sections_with_profile(
            project.path(),
            &profile(4, "Check queue A.", "Main thread."),
            None,
            &[],
        );
        let second = build_hive_context_sections_with_profile(
            project.path(),
            &profile(5, "Check queue B.", "Mobile push."),
            None,
            &[],
        );
        let stable = |sections: &[String]| {
            sections
                .iter()
                .filter(|section| {
                    section.starts_with("[HIVE SOUL")
                        || section.starts_with("[HIVE IDENTITY")
                        || section.starts_with("[HIVE USER")
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let volatile = |sections: &[String]| {
            sections
                .iter()
                .filter(|section| {
                    section.starts_with("[HIVE HEARTBEAT") || section.starts_with("[HIVE CHANNELS")
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        assert_eq!(stable(&first), stable(&second));
        assert_ne!(volatile(&first), volatile(&second));
    }
}
