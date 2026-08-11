//! Typed Codex app-server MCP resource-reading contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MCP_APP_HTML_MIME_TYPE: &str = "text/html;profile=mcp-app";
pub const MCP_APP_SKYBRIDGE_MIME_TYPE: &str = "text/html+skybridge";
pub const MCP_APP_MAX_HTML_BYTES: usize = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub server: String,
    pub uri: String,
}

impl McpResourceReadParams {
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            thread_id: None,
            server: server.into(),
            uri: uri.into(),
        }
    }
}

/// Text or binary contents returned by an MCP resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpResourceContent {
    Text {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Blob {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        blob: String,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadResponse {
    pub contents: Vec<McpResourceContent>,
}

/// Exact MCP app identity retained from a `mcpToolCall` thread item. Keeping
/// this separate from the presentation summary prevents hydration from losing
/// the resource URI, tool inputs, or result needed by the sandbox host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAppToolCall {
    pub server: String,
    pub tool: String,
    pub resource_uri: String,
    pub arguments: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub connector_id: Option<String>,
    pub app_name: Option<String>,
    pub action_name: Option<String>,
    pub link_id: Option<String>,
    pub plugin_id: Option<String>,
}

impl McpAppToolCall {
    pub fn from_thread_item(item: &Value) -> Option<Self> {
        if item.get("type")?.as_str()? != "mcpToolCall" {
            return None;
        }
        let app_context = item.get("appContext").and_then(Value::as_object);
        let resource_uri = app_context
            .and_then(|context| context.get("resourceUri"))
            .and_then(Value::as_str)
            .or_else(|| item.get("mcpAppResourceUri").and_then(Value::as_str))?
            .trim();
        if resource_uri.is_empty() {
            return None;
        }
        let context_text = |key: &str| {
            app_context
                .and_then(|context| context.get(key))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            server: item.get("server")?.as_str()?.to_owned(),
            tool: item.get("tool")?.as_str()?.to_owned(),
            resource_uri: resource_uri.to_owned(),
            arguments: item.get("arguments")?.clone(),
            result: item.get("result").filter(|value| !value.is_null()).cloned(),
            error: item
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            connector_id: context_text("connectorId"),
            app_name: context_text("appName"),
            action_name: context_text("actionName"),
            link_id: context_text("linkId"),
            plugin_id: item
                .get("pluginId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAppHtmlResource {
    pub uri: String,
    pub html: String,
    pub meta: Option<Value>,
}

/// Network and embedding allowlist declared by an MCP App resource. Empty
/// lists intentionally mean deny-by-default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpAppSandboxPolicy {
    pub connect_domains: Vec<String>,
    pub resource_domains: Vec<String>,
    pub frame_domains: Vec<String>,
    pub base_uri_domains: Vec<String>,
}

impl McpAppHtmlResource {
    pub fn sandbox_policy(&self) -> McpAppSandboxPolicy {
        let ui = self.meta.as_ref().and_then(|meta| meta.get("ui"));
        let csp = ui.and_then(|ui| ui.get("csp")).or_else(|| {
            self.meta
                .as_ref()
                .and_then(|meta| meta.get("openai/widgetCSP"))
        });
        McpAppSandboxPolicy {
            connect_domains: csp_domains(csp, &["connectDomains", "connect_domains"]),
            resource_domains: csp_domains(csp, &["resourceDomains", "resource_domains"]),
            frame_domains: csp_domains(csp, &["frameDomains", "frame_domains"]),
            base_uri_domains: csp_domains(csp, &["baseUriDomains", "base_uri_domains"]),
        }
    }

    /// Apply the declared allowlist before the HTML reaches WebKit. A second
    /// CSP meta policy can only make an app more restrictive, never loosen
    /// this host-owned policy.
    pub fn sandboxed_html(&self) -> String {
        let policy = self.sandbox_policy();
        let resource_sources = csp_sources(&policy.resource_domains, "'none'");
        let connect_sources = csp_sources(&policy.connect_domains, "'none'");
        let frame_sources = csp_sources(&policy.frame_domains, "'none'");
        let base_sources = csp_sources(&policy.base_uri_domains, "'none'");
        let csp = format!(
            "default-src 'none'; script-src 'unsafe-inline' {resource_sources}; style-src 'unsafe-inline' {resource_sources}; img-src data: blob: {resource_sources}; font-src data: {resource_sources}; media-src data: blob: {resource_sources}; connect-src {connect_sources}; frame-src {frame_sources}; base-uri {base_sources}; object-src 'none'; form-action 'none'"
        );
        let meta = format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
            csp.replace('&', "&amp;").replace('"', "&quot;")
        );
        inject_head_markup(&self.html, &meta)
    }
}

fn csp_domains(csp: Option<&Value>, keys: &[&str]) -> Vec<String> {
    let mut domains = Vec::new();
    for key in keys {
        let Some(values) = csp.and_then(|csp| csp.get(key)).and_then(Value::as_array) else {
            continue;
        };
        for value in values.iter().filter_map(Value::as_str) {
            let value = value.trim();
            if valid_csp_source(value) && !domains.iter().any(|domain| domain == value) {
                domains.push(value.to_owned());
            }
        }
    }
    domains
}

fn valid_csp_source(source: &str) -> bool {
    !source.is_empty()
        && !source.bytes().any(|byte| {
            byte.is_ascii_whitespace() || matches!(byte, b'\'' | b'"' | b';' | b'<' | b'>')
        })
        && (source == "data:"
            || source == "blob:"
            || source.starts_with("https://")
            || source.starts_with("wss://"))
}

fn csp_sources(domains: &[String], fallback: &str) -> String {
    if domains.is_empty() {
        fallback.to_owned()
    } else {
        domains.join(" ")
    }
}

fn inject_head_markup(html: &str, markup: &str) -> String {
    let lowercase = html.to_ascii_lowercase();
    if let Some(head_start) = lowercase.find("<head") {
        if let Some(relative_end) = lowercase[head_start..].find('>') {
            let insertion = head_start + relative_end + 1;
            let mut output = String::with_capacity(html.len() + markup.len());
            output.push_str(&html[..insertion]);
            output.push_str(markup);
            output.push_str(&html[insertion..]);
            return output;
        }
    }
    format!("<!doctype html><html><head>{markup}</head><body>{html}</body></html>")
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpAppResourceError {
    #[error("MCP app returned no HTML content")]
    MissingHtml,
    #[error("MCP app HTML exceeds the 10 MB limit")]
    TooLarge,
    #[error("MCP app resource uses unsupported MIME type {0}")]
    UnsupportedMimeType(String),
}

impl McpResourceReadResponse {
    /// Select the reference-compatible HTML content for one MCP app. Blob
    /// resources and generic HTML are not executed as apps, and the same 10 MB
    /// ceiling as the reviewed desktop is enforced before a sandbox sees HTML.
    pub fn into_mcp_app_html(
        self,
        expected_uri: &str,
    ) -> Result<McpAppHtmlResource, McpAppResourceError> {
        let mut unsupported_mime = None;
        for content in self.contents {
            let McpResourceContent::Text {
                uri,
                mime_type,
                text,
                meta,
            } = content
            else {
                continue;
            };
            if uri != expected_uri {
                continue;
            }
            let mime_type = mime_type.unwrap_or_default();
            if !matches!(
                mime_type.as_str(),
                MCP_APP_HTML_MIME_TYPE | MCP_APP_SKYBRIDGE_MIME_TYPE
            ) {
                if !mime_type.is_empty() {
                    unsupported_mime = Some(mime_type);
                }
                continue;
            }
            if text.len() > MCP_APP_MAX_HTML_BYTES {
                return Err(McpAppResourceError::TooLarge);
            }
            return Ok(McpAppHtmlResource {
                uri,
                html: text,
                meta,
            });
        }
        match unsupported_mime {
            Some(mime) => Err(McpAppResourceError::UnsupportedMimeType(mime)),
            None => Err(McpAppResourceError::MissingHtml),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_response_preserves_text_blob_and_metadata() {
        let response: McpResourceReadResponse = serde_json::from_value(serde_json::json!({
            "contents": [
                {"uri": "docs://one", "mimeType": "text/plain", "text": "hello"},
                {"uri": "asset://two", "blob": "AQI=", "_meta": {"source": "live"}}
            ]
        }))
        .unwrap();
        assert!(matches!(
            response.contents[0],
            McpResourceContent::Text { .. }
        ));
        assert!(matches!(
            response.contents[1],
            McpResourceContent::Blob { .. }
        ));
    }

    #[test]
    fn thread_item_prefers_current_app_context_and_preserves_invocation_state() {
        let call = McpAppToolCall::from_thread_item(&serde_json::json!({
            "type": "mcpToolCall",
            "id": "call-1",
            "server": "calendar",
            "tool": "find_events",
            "arguments": {"day": "Monday"},
            "status": "completed",
            "appContext": {
                "connectorId": "connector-calendar",
                "resourceUri": "ui://calendar/current",
                "appName": "Calendar",
                "actionName": "Find events",
                "linkId": "link-1"
            },
            "mcpAppResourceUri": "ui://legacy",
            "pluginId": "plugin-1",
            "result": {"content": [], "structuredContent": {"count": 2}}
        }))
        .unwrap();
        assert_eq!(call.resource_uri, "ui://calendar/current");
        assert_eq!(call.connector_id.as_deref(), Some("connector-calendar"));
        assert_eq!(call.arguments, serde_json::json!({"day": "Monday"}));
        assert_eq!(
            call.result.unwrap()["structuredContent"],
            serde_json::json!({"count": 2})
        );
    }

    #[test]
    fn app_html_requires_reference_mime_and_size_contract() {
        let response: McpResourceReadResponse = serde_json::from_value(serde_json::json!({
            "contents": [{
                "uri": "ui://calendar/current",
                "mimeType": "text/html;profile=mcp-app",
                "text": "<main>Calendar</main>",
                "_meta": {"ui": {"prefersBorder": true}}
            }]
        }))
        .unwrap();
        let resource = response.into_mcp_app_html("ui://calendar/current").unwrap();
        assert_eq!(resource.html, "<main>Calendar</main>");
        assert_eq!(resource.meta.unwrap()["ui"]["prefersBorder"], true);

        let wrong_mime = McpResourceReadResponse {
            contents: vec![McpResourceContent::Text {
                uri: "ui://calendar/current".to_owned(),
                mime_type: Some("text/html".to_owned()),
                text: "<main>Unsafe generic HTML</main>".to_owned(),
                meta: None,
            }],
        };
        assert_eq!(
            wrong_mime.into_mcp_app_html("ui://calendar/current"),
            Err(McpAppResourceError::UnsupportedMimeType(
                "text/html".to_owned()
            ))
        );

        let too_large = McpResourceReadResponse {
            contents: vec![McpResourceContent::Text {
                uri: "ui://calendar/current".to_owned(),
                mime_type: Some(MCP_APP_HTML_MIME_TYPE.to_owned()),
                text: "x".repeat(MCP_APP_MAX_HTML_BYTES + 1),
                meta: None,
            }],
        };
        assert_eq!(
            too_large.into_mcp_app_html("ui://calendar/current"),
            Err(McpAppResourceError::TooLarge)
        );
    }

    #[test]
    fn sandbox_policy_is_deny_by_default_and_filters_injected_sources() {
        let resource = McpAppHtmlResource {
            uri: "ui://calendar/current".to_owned(),
            html: "<html><head><title>Calendar</title></head><body>ready</body></html>".to_owned(),
            meta: Some(serde_json::json!({
                "ui": {"csp": {
                    "connectDomains": ["https://api.example.com", "https://api.example.com", "*; script-src *"],
                    "resourceDomains": ["https://cdn.example.com"],
                    "frameDomains": ["https://embed.example.com"],
                    "baseUriDomains": ["https://base.example.com"]
                }}
            })),
        };
        let policy = resource.sandbox_policy();
        assert_eq!(policy.connect_domains, vec!["https://api.example.com"]);
        assert_eq!(policy.resource_domains, vec!["https://cdn.example.com"]);
        assert_eq!(policy.frame_domains, vec!["https://embed.example.com"]);
        assert_eq!(policy.base_uri_domains, vec!["https://base.example.com"]);
        let html = resource.sandboxed_html();
        assert!(html.contains("connect-src https://api.example.com"));
        assert!(html.contains("object-src 'none'"));
        assert!(!html.contains("*; script-src *"));

        let deny_all = McpAppHtmlResource {
            uri: "ui://empty".to_owned(),
            html: "<main>empty</main>".to_owned(),
            meta: None,
        }
        .sandboxed_html();
        assert!(deny_all.contains("connect-src 'none'"));
        assert!(deny_all.contains("frame-src 'none'"));
        assert!(deny_all.contains("base-uri 'none'"));
    }
}
