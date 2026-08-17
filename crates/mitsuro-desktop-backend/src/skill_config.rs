//! Typed Codex skill configuration mutation contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfigWriteParams {
    pub path: Option<String>,
    pub name: Option<String>,
    pub enabled: bool,
}

impl SkillsConfigWriteParams {
    pub fn for_skill(path: impl Into<String>, name: impl Into<String>, enabled: bool) -> Self {
        let path = path.into();
        let name = name.into();
        Self {
            path: (!path.trim().is_empty()).then_some(path),
            name: (!name.trim().is_empty()).then_some(name),
            enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfigWriteResponse {
    pub effective_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_params_match_generated_selector_shape() {
        assert_eq!(
            serde_json::to_value(SkillsConfigWriteParams::for_skill(
                "/home/test/.codex/skills/review/SKILL.md",
                "review",
                false,
            ))
            .unwrap(),
            serde_json::json!({
                "path": "/home/test/.codex/skills/review/SKILL.md",
                "name": "review",
                "enabled": false
            })
        );
    }
}
