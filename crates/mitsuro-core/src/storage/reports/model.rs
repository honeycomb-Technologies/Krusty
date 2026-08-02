use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub project_dir: Option<String>,
    pub content: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: String,
}

pub struct CreateReportInput<'a> {
    pub title: &'a str,
    pub session_id: &'a str,
    pub project_dir: Option<&'a str>,
    pub report_root: Option<&'a Path>,
    pub content: &'a str,
    pub summary: &'a str,
    pub tags: &'a [String],
    pub sources: &'a [String],
}
