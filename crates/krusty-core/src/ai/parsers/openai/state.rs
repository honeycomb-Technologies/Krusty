use std::collections::HashMap;

use serde_json::Value;

use super::OpenAIParser;
use crate::ai::sse::ToolCallAccumulator;
use crate::ai::types::AiToolCall;

impl OpenAIParser {
    /// Lock tool accumulators with proper error handling
    pub(super) fn lock_tool_accumulators(
        &self,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, HashMap<String, ToolCallAccumulator>>> {
        self.tool_accumulators
            .lock()
            .map_err(|e| anyhow::anyhow!("Tool accumulators lock poisoned: {}", e))
    }

    pub(super) fn lock_tool_order(&self) -> anyhow::Result<std::sync::MutexGuard<'_, Vec<String>>> {
        self.tool_order
            .lock()
            .map_err(|e| anyhow::anyhow!("Tool order lock poisoned: {}", e))
    }

    pub(super) fn lock_response_item_map(
        &self,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, HashMap<String, String>>> {
        self.response_item_to_call
            .lock()
            .map_err(|e| anyhow::anyhow!("Response item map lock poisoned: {}", e))
    }

    pub(super) fn register_tool_call(
        &self,
        key: String,
        id: &str,
        name: &str,
        item_id: Option<String>,
    ) -> anyhow::Result<bool> {
        let mut inserted = false;
        {
            let mut accumulators = self.lock_tool_accumulators()?;
            if !accumulators.contains_key(&key) {
                accumulators.insert(
                    key.clone(),
                    ToolCallAccumulator::new(id.to_string(), name.to_string()),
                );
                inserted = true;
            }
        }

        if inserted {
            let mut order = self.lock_tool_order()?;
            if !order.contains(&key) {
                order.push(key.clone());
            }
        }

        if let Some(item_id) = item_id {
            if !item_id.is_empty() {
                let mut map = self.lock_response_item_map()?;
                map.insert(item_id, key);
            }
        }

        Ok(inserted)
    }

    pub(super) fn append_tool_arguments(
        &self,
        key: &str,
        delta: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut accumulators = self.lock_tool_accumulators()?;
        if let Some(acc) = accumulators.get_mut(key) {
            acc.add_arguments(delta);
            return Ok(Some(acc.id.clone()));
        }
        Ok(None)
    }

    pub(super) fn apply_tool_arguments_snapshot(
        &self,
        key: &str,
        snapshot: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut accumulators = self.lock_tool_accumulators()?;
        if let Some(acc) = accumulators.get_mut(key) {
            let mut replace_snapshot = acc.arguments.is_empty();
            if !replace_snapshot && acc.arguments != snapshot {
                if snapshot.len() > acc.arguments.len() && snapshot.starts_with(&acc.arguments) {
                    replace_snapshot = true;
                } else {
                    let current_valid = serde_json::from_str::<Value>(&acc.arguments).is_ok();
                    let snapshot_valid = serde_json::from_str::<Value>(snapshot).is_ok();
                    if !current_valid && snapshot_valid {
                        replace_snapshot = true;
                    }
                }
            }

            if replace_snapshot {
                acc.arguments.clear();
                acc.arguments.push_str(snapshot);
            }

            return Ok(Some(acc.id.clone()));
        }
        Ok(None)
    }

    pub(super) fn resolve_responses_tool_key(
        &self,
        json: &Value,
    ) -> anyhow::Result<Option<String>> {
        if let Some(call_id) = json
            .get("call_id")
            .and_then(|id| id.as_str())
            .filter(|id| !id.is_empty())
        {
            return Ok(Some(call_id.to_string()));
        }

        if let Some(item) = json.get("item") {
            if let Some(call_id) = item
                .get("call_id")
                .and_then(|id| id.as_str())
                .filter(|id| !id.is_empty())
            {
                return Ok(Some(call_id.to_string()));
            }
            if let Some(item_id) = item
                .get("id")
                .and_then(|id| id.as_str())
                .filter(|id| !id.is_empty())
            {
                if let Some(key) = self.lock_response_item_map()?.get(item_id).cloned() {
                    return Ok(Some(key));
                }
            }
        }

        if let Some(item_id) = json
            .get("item_id")
            .and_then(|id| id.as_str())
            .filter(|id| !id.is_empty())
        {
            if let Some(key) = self.lock_response_item_map()?.get(item_id).cloned() {
                return Ok(Some(key));
            }
        }

        let accumulators = self.lock_tool_accumulators()?;
        if accumulators.len() == 1 {
            return Ok(accumulators.keys().next().cloned());
        }

        Ok(None)
    }

    pub(super) fn drain_tool_calls(&self) -> anyhow::Result<Vec<AiToolCall>> {
        let keys = {
            let mut order = self.lock_tool_order()?;
            std::mem::take(&mut *order)
        };

        let mut accumulators = self.lock_tool_accumulators()?;
        let mut tool_calls = Vec::new();

        for key in keys {
            if let Some(mut acc) = accumulators.remove(&key) {
                tool_calls.push(acc.force_complete());
            }
        }

        for (_, mut acc) in accumulators.drain() {
            tool_calls.push(acc.force_complete());
        }

        self.lock_response_item_map()?.clear();
        Ok(tool_calls)
    }
}
