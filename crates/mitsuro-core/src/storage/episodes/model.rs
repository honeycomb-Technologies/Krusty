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
    pub limit: usize,
}

impl<'a> EpisodeSearch<'a> {
    pub fn new(query: &'a str, user_id: Option<&'a str>) -> Self {
        Self {
            query,
            user_id,
            project_dir: None,
            limit: 20,
        }
    }
}
