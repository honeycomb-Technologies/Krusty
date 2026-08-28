use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationEpisode {
    pub id: i64,
    pub session_id: String,
    pub source_message_id: Option<i64>,
    pub role: String,
    pub body: String,
    pub content_hash: String,
    pub occurred_at: String,
    pub session_title: String,
    pub project_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeSearch<'a> {
    pub query: &'a str,
    pub user_id: Option<&'a str>,
    pub project_dir: Option<&'a str>,
    pub session_type: Option<&'a str>,
    pub session_id: Option<&'a str>,
    /// When set, search only this Worker's durable DM and group-lane
    /// sessions. This is the prompt-time path for Worker continuity; it can
    /// never broaden into another Worker's or an ordinary Hive session.
    pub worker_id: Option<&'a str>,
    /// Explicit diagnostic escape hatch. Prompt-time searches keep this
    /// false so Worker DM/group episodes never enter owner-wide recall. It is
    /// ignored when `worker_id` is set because Worker-scoped reads stay exact.
    pub include_worker_sessions: bool,
    pub limit: usize,
}

impl<'a> EpisodeSearch<'a> {
    pub fn new(query: &'a str, user_id: Option<&'a str>) -> Self {
        Self {
            query,
            user_id,
            project_dir: None,
            session_type: None,
            session_id: None,
            worker_id: None,
            include_worker_sessions: false,
            limit: 20,
        }
    }
}
