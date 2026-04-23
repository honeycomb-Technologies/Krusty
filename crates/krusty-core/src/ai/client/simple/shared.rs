use serde_json::Value;

pub(super) fn trim_or_empty(text: Option<&str>) -> String {
    text.unwrap_or("").trim().to_string()
}

pub(super) fn collect_anthropic_text(blocks: &[Value]) -> String {
    let mut text = String::new();
    for block in blocks {
        if block.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(chunk) = block.get("text").and_then(|t| t.as_str()) {
            text.push_str(chunk);
        }
    }
    text
}
