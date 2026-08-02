use crate::ai::client::core::MITSURO_SYSTEM_PROMPT;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

use super::ModelProfile;

impl ModelProfile {
    pub fn layered_system_prompt(
        self,
        _provider: ProviderId,
        _api_format: ApiFormat,
        _model_id: &str,
        custom_system_prompt: Option<&str>,
    ) -> String {
        if let Some(custom) = custom_system_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return custom.to_string();
        }

        // Shared coding contract only. Model-family prompt overlays were
        // intentionally removed so providers share one stable instruction prefix.
        MITSURO_SYSTEM_PROMPT.to_string()
    }
}
