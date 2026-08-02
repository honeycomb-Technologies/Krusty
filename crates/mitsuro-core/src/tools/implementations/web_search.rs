//! Local web search tool.

use std::collections::HashSet;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::ai::types::WebSearchResult;
use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::web_utils::{
    decode_html_entities, get_limited, strip_tags_to_text, tool_error, web_client,
};

const DEFAULT_MAX_RESULTS: usize = 5;
const HARD_MAX_RESULTS: usize = 10;
const SEARCH_MAX_BYTES: usize = 500_000;
const DUCKDUCKGO_HTML_ENDPOINT: &str = "https://duckduckgo.com/html/";

static ANCHOR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<a\b[^>]*>.*?</a>").expect("anchor regex should compile"));
static HREF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
        .expect("href regex should compile")
});

pub struct WebSearchTool;

#[derive(Deserialize)]
struct Params {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the live web and return source URLs with titles. Use for current information or when the answer depends on online sources."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use for current, changing, or source-backed questions. Search results contain URLs and titles; use web_fetch on relevant results when you need page details.

Prefer concise targeted queries. Cite sources in your final answer when you rely on web_search/web_fetch."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default 5, max 10)"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let query = params.query.trim();
        if query.is_empty() {
            return ToolResult::invalid_parameters("Search query cannot be empty");
        }

        let max_results = params
            .max_results
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .clamp(1, HARD_MAX_RESULTS);
        let search_url = match Url::parse_with_params(DUCKDUCKGO_HTML_ENDPOINT, &[("q", query)]) {
            Ok(url) => url,
            Err(err) => return tool_error(err),
        };
        let client = match web_client() {
            Ok(client) => client,
            Err(err) => return tool_error(err),
        };

        let (_final_url, content_type, bytes) =
            match get_limited(&client, search_url, SEARCH_MAX_BYTES).await {
                Ok(response) => response,
                Err(err) => return tool_error(err),
            };
        let html = match String::from_utf8(bytes) {
            Ok(html) => html,
            Err(err) => return tool_error(format!("Search response was not valid UTF-8: {err}")),
        };
        let results = parse_duckduckgo_results(&html, max_results);

        ToolResult::success_data(json!({
            "query": query,
            "results": results,
            "result_count": results.len(),
            "media_type": content_type,
            "source": "duckduckgo_html"
        }))
    }
}

fn parse_duckduckgo_results(html: &str, max_results: usize) -> Vec<WebSearchResult> {
    let mut seen = HashSet::new();
    let mut results = Vec::new();

    for anchor_match in ANCHOR_RE.find_iter(html) {
        let anchor = anchor_match.as_str();
        if !anchor.contains("result__a") {
            continue;
        }

        let Some(raw_href) = extract_href(anchor) else {
            continue;
        };
        let Some(url) = normalize_result_url(&raw_href) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }

        let title = strip_tags_to_text(anchor);
        if title.is_empty() {
            continue;
        }

        results.push(WebSearchResult {
            url,
            title,
            encrypted_content: None,
            page_age: None,
        });

        if results.len() >= max_results {
            break;
        }
    }

    results
}

fn extract_href(anchor: &str) -> Option<String> {
    let caps = HREF_RE.captures(anchor)?;
    caps.get(1)
        .or_else(|| caps.get(2))
        .or_else(|| caps.get(3))
        .map(|value| decode_html_entities(value.as_str()))
}

fn normalize_result_url(raw_href: &str) -> Option<String> {
    let href = if raw_href.starts_with("//") {
        format!("https:{raw_href}")
    } else if raw_href.starts_with('/') {
        format!("https://duckduckgo.com{raw_href}")
    } else {
        raw_href.to_string()
    };

    let parsed = Url::parse(&href).ok()?;
    let host = parsed.host_str().unwrap_or_default();
    if host.ends_with("duckduckgo.com") {
        if let Some((_, value)) = parsed.query_pairs().find(|(key, _)| key == "uddg") {
            return validate_result_url(value.as_ref());
        }
        return None;
    }

    validate_result_url(parsed.as_str())
}

fn validate_result_url(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duckduckgo_result_links() {
        let html = r#"
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdoc&amp;rut=abc">Example &amp; Docs</a>
            <a rel="nofollow" class="result__a" href="https://example.org/other">Other</a>
        "#;

        let results = parse_duckduckgo_results(html, 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example.com/doc");
        assert_eq!(results[0].title, "Example & Docs");
        assert_eq!(results[1].url, "https://example.org/other");
    }

    #[test]
    fn schema_uses_expected_tool_name() {
        assert_eq!(WebSearchTool.name(), "web_search");
        let schema = WebSearchTool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("query")));
    }
}
