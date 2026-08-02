use std::path::Path;

use tracing::warn;

use crate::agent::context_ledger::ContextLedger;
use crate::ai::types::ModelMessage;
use crate::storage::{EpisodeSearch, EpisodeStore};

use super::{open_context_database, truncate_utf8, truncate_utf8_bytes};

const MAX_EPISODES: usize = 3;
const MAX_EPISODE_PREVIEW_CHARS: usize = 420;
const MAX_EPISODE_CONTEXT_BYTES: usize = 2 * 1024;

pub(super) fn build_episode_context(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
    current_session_id: &str,
    conversation: &[ModelMessage],
) -> String {
    let Some(objective) = ContextLedger::from_conversation(conversation).latest_user_objective
    else {
        return String::new();
    };
    if objective.split_whitespace().count() < 2 {
        return String::new();
    }

    let Some(db) = open_context_database(db_path, "building Hive episodic recall context") else {
        return String::new();
    };
    let store = EpisodeStore::new(&db);
    let mut search = EpisodeSearch::new(&objective, user_id);
    search.project_dir = project_dir;
    search.limit = MAX_EPISODES + 2;
    let episodes = match store.search(&search) {
        Ok(episodes) => episodes,
        Err(error) => {
            warn!(error = %error, "Failed to search Hive conversation episodes");
            return String::new();
        }
    };
    let episodes = episodes
        .into_iter()
        .filter(|episode| episode.session_id != current_session_id)
        .take(MAX_EPISODES)
        .collect::<Vec<_>>();
    if episodes.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        "[RECALLED CONVERSATION EPISODES]".to_string(),
        "These are bounded text-only excerpts from prior owned conversations. Use them as fallible continuity cues, not as proof of current external state. Never imply memory beyond what is shown.".to_string(),
    ];
    for episode in episodes {
        lines.push(format!(
            "- [{} | {} | {}] {}",
            episode.occurred_at,
            episode.session_title,
            episode.role,
            truncate_utf8(&episode.body, MAX_EPISODE_PREVIEW_CHARS),
        ));
    }
    lines.push("[/RECALLED CONVERSATION EPISODES]".to_string());
    truncate_utf8_bytes(&lines.join("\n"), MAX_EPISODE_CONTEXT_BYTES)
}
