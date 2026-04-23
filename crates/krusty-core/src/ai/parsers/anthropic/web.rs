use serde_json::Value;

use super::AnthropicParser;
use crate::ai::types::{Citation, WebFetchContent, WebSearchResult};

impl AnthropicParser {
    /// Parse web search results from content block
    pub(super) fn parse_search_results(&self, content_block: &Value) -> Vec<WebSearchResult> {
        let mut results = Vec::new();

        if let Some(content_arr) = content_block.get("content").and_then(|c| c.as_array()) {
            for item in content_arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("web_search_result") {
                    let url = item
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string();
                    let title = item
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let encrypted_content = item
                        .get("encrypted_content")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string());
                    let page_age = item
                        .get("page_age")
                        .and_then(|p| p.as_str())
                        .map(|s| s.to_string());

                    results.push(WebSearchResult {
                        url,
                        title,
                        encrypted_content,
                        page_age,
                    });
                }
            }
        }

        results
    }

    /// Parse web fetch result from content block
    pub(super) fn parse_fetch_result(&self, content_block: &Value) -> Option<WebFetchContent> {
        let content = content_block.get("content")?;

        if content.get("type").and_then(|t| t.as_str()) != Some("web_fetch_result") {
            return None;
        }

        let url = content
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        let retrieved_at = content
            .get("retrieved_at")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        if let Some(doc) = content.get("content") {
            let title = doc
                .get("title")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            if let Some(source) = doc.get("source") {
                let media_type = source
                    .get("media_type")
                    .and_then(|m| m.as_str())
                    .unwrap_or("text/plain")
                    .to_string();

                let content_data = source
                    .get("data")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();

                return Some(WebFetchContent {
                    url,
                    content: content_data,
                    media_type,
                    title,
                    retrieved_at,
                });
            }
        }

        None
    }

    /// Parse citations from a text delta
    pub(super) fn parse_citations(&self, citations_arr: &[Value]) -> Vec<Citation> {
        citations_arr
            .iter()
            .filter_map(|c| {
                let url = c
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let title = c
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let cited_text = c
                    .get("cited_text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                if url.is_empty() && title.is_empty() {
                    None
                } else {
                    Some(Citation {
                        url,
                        title,
                        cited_text,
                    })
                }
            })
            .collect()
    }
}
