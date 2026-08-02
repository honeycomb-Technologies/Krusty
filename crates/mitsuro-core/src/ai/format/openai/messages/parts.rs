use serde_json::Value;

use super::super::OpenAIFormat;
use crate::ai::types::ImageContent;

/// Convert image content to a URL accepted by OpenAI:
/// - pass-through remote URL
/// - data URL for base64 payloads
fn image_to_url(image: &ImageContent) -> Option<String> {
    if let Some(url) = &image.url {
        return Some(url.clone());
    }

    image.base64.as_ref().map(|base64| {
        let media_type = image.media_type.as_deref().unwrap_or("image/png");
        format!("data:{};base64,{}", media_type, base64)
    })
}

/// Build a user text content part for the current OpenAI API flavor.
pub(super) fn user_text_part(format: &OpenAIFormat, text: &str) -> Value {
    if format.is_responses_format() {
        serde_json::json!({
            "type": "input_text",
            "text": text
        })
    } else {
        serde_json::json!({
            "type": "text",
            "text": text
        })
    }
}

/// Build a user image content part for the current OpenAI API flavor.
pub(super) fn user_image_part(
    format: &OpenAIFormat,
    image: &ImageContent,
    detail: Option<&str>,
) -> Option<Value> {
    let image_url = image_to_url(image)?;
    let detail = detail.filter(|d| !d.is_empty());

    if format.is_responses_format() {
        let mut part = serde_json::json!({
            "type": "input_image",
            "image_url": image_url
        });
        if let Some(detail) = detail {
            part["detail"] = serde_json::json!(detail);
        }
        Some(part)
    } else {
        let mut image_url_obj = serde_json::json!({
            "url": image_url
        });
        if let Some(detail) = detail {
            image_url_obj["detail"] = serde_json::json!(detail);
        }
        Some(serde_json::json!({
            "type": "image_url",
            "image_url": image_url_obj
        }))
    }
}
