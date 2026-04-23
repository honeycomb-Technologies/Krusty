use serde_json::Value;

/// Extract text content from an Anthropic-format content array
pub fn extract_text_from_content(content: Option<&Value>) -> String {
    let mut text = String::new();
    if let Some(arr) = content.and_then(|c| c.as_array()) {
        for item in arr {
            if item.get("type").and_then(|t| t.as_str()) != Some("text") {
                continue;
            }
            if let Some(chunk) = item.get("text").and_then(|t| t.as_str()) {
                text.push_str(chunk);
            }
        }
    }
    text
}
