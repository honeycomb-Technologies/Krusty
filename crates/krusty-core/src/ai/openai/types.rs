use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ModelsResponse {
    pub(super) data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiModel {
    pub(super) id: String,
}
