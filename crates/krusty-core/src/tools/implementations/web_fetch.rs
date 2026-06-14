//! Local web fetch tool.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::registry::Tool;
use crate::tools::{parse_params, ToolContext, ToolResult};

use super::web_utils::{
    bytes_to_text, get_limited, html_title, html_to_text, is_html, is_text_like, tool_error,
    validate_http_url, web_client,
};

const DEFAULT_MAX_BYTES: usize = 250_000;
const HARD_MAX_BYTES: usize = 2_000_000;

pub struct WebFetchTool;

#[derive(Deserialize)]
struct Params {
    url: String,
    #[serde(default)]
    max_bytes: Option<usize>,
    #[serde(default)]
    raw: bool,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a live HTTP/HTTPS URL and return readable text. Supports HTML and text-like content. Use after web_search or when the user provides a specific URL."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use for current web pages or user-provided URLs. Prefer web_search first when you need to discover sources.

Only fetch URLs that are relevant to the user's request. The local fallback extracts readable text from HTML/text; PDFs and JavaScript-rendered pages may require a provider-hosted web tool or browser-capable workflow."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to fetch"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum response bytes to download (default 250000, hard max 2000000)"
                },
                "raw": {
                    "type": "boolean",
                    "description": "Return raw text/HTML instead of extracting readable text from HTML"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let url = match validate_http_url(&params.url) {
            Ok(url) => url,
            Err(err) => return ToolResult::invalid_parameters(err),
        };
        let max_bytes = params
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(HARD_MAX_BYTES);

        let client = match web_client() {
            Ok(client) => client,
            Err(err) => return tool_error(err),
        };

        let (final_url, content_type, bytes) = match get_limited(&client, url, max_bytes).await {
            Ok(response) => response,
            Err(err) => return tool_error(err),
        };
        let byte_len = bytes.len();

        if !is_text_like(&content_type) && !is_html(&content_type, &final_url) {
            return ToolResult::error_with_details(
                "unsupported_content_type",
                format!(
                    "Unsupported content type: {content_type}. Local web_fetch supports HTML and text-like responses."
                ),
                Some(json!({
                    "url": final_url.as_str(),
                    "content_type": content_type,
                    "bytes": byte_len
                })),
                Some(json!({ "source": "local_web_tool" })),
            );
        }

        let raw_text = match bytes_to_text(bytes, &content_type) {
            Ok(text) => text,
            Err(err) => return tool_error(err),
        };
        let html = is_html(&content_type, &final_url);
        let title = html.then(|| html_title(&raw_text)).flatten();
        let content = if html && !params.raw {
            html_to_text(&raw_text)
        } else {
            raw_text
        };

        ToolResult::success_data(json!({
            "url": final_url.as_str(),
            "title": title,
            "media_type": content_type,
            "content": content,
            "bytes": byte_len,
            "source": "local_web_fetch"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_uses_expected_tool_name() {
        assert_eq!(WebFetchTool.name(), "web_fetch");
        let schema = WebFetchTool.parameters_schema();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("url")));
    }
}
