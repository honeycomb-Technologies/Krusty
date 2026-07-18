use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ModelsResponse {
    #[serde(default)]
    pub(super) data: Vec<MiniMaxModel>,
    #[serde(default)]
    pub(super) last_id: Option<String>,
    #[serde(default)]
    pub(super) has_more: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct MiniMaxModel {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) display_name: Option<String>,
}
