use std::collections::HashSet;

use super::{MakoChannelBinding, MakoChannelKind, MakoHomeDocument, MakoHomeProfile};

pub fn summarize_channel_bindings(profile: &MakoHomeProfile) -> Vec<MakoChannelBinding> {
    let mut bindings = vec![MakoChannelBinding {
        id: "main-thread".to_string(),
        label: "Main thread".to_string(),
        kind: MakoChannelKind::MainThread,
        enabled: true,
        detail: "Primary Hive conversation inside Mitsuro.".to_string(),
        source: "system",
    }];

    if !profile.crew.is_empty() {
        bindings.push(MakoChannelBinding {
            id: "crew-handoff".to_string(),
            label: "Agent handoff".to_string(),
            kind: MakoChannelKind::Crew,
            enabled: true,
            detail: format!(
                "{} Hive agent{} can route updates back to this thread.",
                profile.crew.len(),
                if profile.crew.len() == 1 { "" } else { "s" }
            ),
            source: "system",
        });
    }

    let mut seen = bindings
        .iter()
        .map(|binding| binding.id.clone())
        .collect::<HashSet<_>>();

    if let Some(document) = profile.channels.as_ref() {
        for binding in parse_channel_document(document) {
            if seen.insert(binding.id.clone()) {
                bindings.push(binding);
            }
        }
    }

    bindings
}

fn parse_channel_document(document: &MakoHomeDocument) -> Vec<MakoChannelBinding> {
    document
        .content
        .lines()
        .filter_map(parse_channel_line)
        .collect()
}

fn parse_channel_line(line: &str) -> Option<MakoChannelBinding> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with('>')
        || trimmed.starts_with("```")
    {
        return None;
    }

    let mut enabled = !trimmed.contains("[ ]");
    let body = trimmed
        .trim_start_matches("- [x]")
        .trim_start_matches("- [X]")
        .trim_start_matches("- [ ]")
        .trim_start_matches("* [x]")
        .trim_start_matches("* [X]")
        .trim_start_matches("* [ ]")
        .trim_start_matches("- ")
        .trim_start_matches("* ")
        .trim_start_matches("+ ")
        .trim();

    if body.is_empty() {
        return None;
    }

    let lower = body.to_ascii_lowercase();
    if lower.contains("disabled") || lower.contains("inactive") || lower.contains("off") {
        enabled = false;
    }
    if lower.contains("enabled") || lower.contains("active") || lower.contains("primary") {
        enabled = true;
    }

    let (label, detail) = if let Some((left, right)) = body.split_once(':') {
        (left.trim(), right.trim())
    } else if let Some((left, right)) = body.split_once('|') {
        (left.trim(), right.trim())
    } else {
        (body, body)
    };

    if label.is_empty() {
        return None;
    }

    let kind = infer_channel_kind(label, detail);
    let id = slugify_channel_label(label);
    if id.is_empty() {
        return None;
    }

    Some(MakoChannelBinding {
        id,
        label: title_case_channel_label(label),
        kind,
        enabled,
        detail: detail.to_string(),
        source: "home",
    })
}

fn infer_channel_kind(label: &str, detail: &str) -> MakoChannelKind {
    let haystack = format!(
        "{} {}",
        label.trim().to_ascii_lowercase(),
        detail.trim().to_ascii_lowercase()
    );

    if haystack.contains("thread") || haystack.contains("chat") {
        MakoChannelKind::MainThread
    } else if haystack.contains("push")
        || haystack.contains("apns")
        || haystack.contains("iphone")
        || haystack.contains("ios")
        || haystack.contains("mobile")
    {
        MakoChannelKind::MobilePush
    } else if haystack.contains("crew") || haystack.contains("agent") {
        MakoChannelKind::Crew
    } else if haystack.contains("webhook") {
        MakoChannelKind::Webhook
    } else if haystack.contains("email") {
        MakoChannelKind::Email
    } else if haystack.contains("web")
        || haystack.contains("desktop")
        || haystack.contains("browser")
    {
        MakoChannelKind::Web
    } else {
        MakoChannelKind::Unknown
    }
}

fn slugify_channel_label(label: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in label.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    slug.trim_matches('-').to_string()
}

fn title_case_channel_label(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut output = String::new();
                    output.push(first.to_ascii_uppercase());
                    output.push_str(chars.as_str());
                    output
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
