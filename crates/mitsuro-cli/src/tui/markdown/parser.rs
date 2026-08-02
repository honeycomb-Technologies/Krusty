//! Markdown parsing using pulldown-cmark

use once_cell::sync::Lazy;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use regex::Regex;

use super::elements::{InlineContent, ListItem, MarkdownElement, TableCell};

/// Regex for detecting bare URLs in text
static URL_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://[^\s<>\[\]()]+").unwrap());

/// Parse markdown text into structured elements
pub fn parse(text: &str) -> Vec<MarkdownElement> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    let parser = Parser::new_ext(text, options);
    let events: Vec<_> = parser.collect();

    parse_events(&events)
}

fn parse_events(events: &[Event<'_>]) -> Vec<MarkdownElement> {
    let mut elements = Vec::new();
    let mut idx = 0;

    while idx < events.len() {
        match &events[idx] {
            Event::Start(Tag::Paragraph) => {
                let (content, new_idx) = parse_inline_until_end(events, idx + 1, TagEnd::Paragraph);
                if !content.is_empty() {
                    elements.push(MarkdownElement::Paragraph(content));
                }
                idx = new_idx;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level_num = *level as u8;
                let end_tag = TagEnd::Heading(*level);
                let (content, new_idx) = parse_inline_until_end(events, idx + 1, end_tag);
                elements.push(MarkdownElement::Heading {
                    level: level_num,
                    content,
                });
                idx = new_idx;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                    _ => None,
                };
                let (code, new_idx) = collect_code_block(events, idx + 1);
                elements.push(MarkdownElement::CodeBlock { lang, code });
                idx = new_idx;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let (nested, new_idx) =
                    parse_block_until_end(events, idx + 1, TagEnd::BlockQuote(None));
                elements.push(MarkdownElement::BlockQuote(nested));
                idx = new_idx;
            }
            Event::Start(Tag::List(start_num)) => {
                let ordered = start_num.is_some();
                let start = *start_num;
                let (items, new_idx) = parse_list_items(events, idx + 1, ordered);
                elements.push(MarkdownElement::List {
                    ordered,
                    start,
                    items,
                });
                idx = new_idx;
            }
            Event::Start(Tag::Table(_)) => {
                let (headers, rows, new_idx) = parse_table(events, idx + 1);
                elements.push(MarkdownElement::Table { headers, rows });
                idx = new_idx;
            }
            Event::Rule => {
                elements.push(MarkdownElement::ThematicBreak);
                idx += 1;
            }
            // Handle loose inline content (tight lists don't wrap items in paragraphs)
            Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Start(Tag::Strong)
            | Event::Start(Tag::Emphasis)
            | Event::Start(Tag::Strikethrough)
            | Event::Start(Tag::Link { .. }) => {
                // Collect consecutive inline events into a paragraph
                let (content, new_idx) = collect_loose_inline(events, idx);
                if !content.is_empty() {
                    elements.push(MarkdownElement::Paragraph(content));
                }
                idx = new_idx;
            }
            _ => {
                idx += 1;
            }
        }
    }

    elements
}

/// Collect loose inline content (text not wrapped in paragraph tags)
/// This handles tight list items where pulldown-cmark doesn't emit paragraph wrappers
fn collect_loose_inline(events: &[Event<'_>], start: usize) -> (Vec<InlineContent>, usize) {
    let mut content = Vec::new();
    let mut idx = start;
    let mut style_stack: Vec<InlineStyle> = Vec::new();

    while idx < events.len() {
        match &events[idx] {
            Event::Text(text) => {
                // Auto-detect URLs in text and convert to links
                for inline in autolink_text(text) {
                    push_with_styles(&mut content, inline, &style_stack);
                }
                idx += 1;
            }
            Event::Code(code) => {
                let inline = InlineContent::Code(code.to_string());
                push_with_styles(&mut content, inline, &style_stack);
                idx += 1;
            }
            Event::Start(Tag::Strong) => {
                style_stack.push(InlineStyle::Bold);
                idx += 1;
            }
            Event::End(TagEnd::Strong) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Emphasis) => {
                style_stack.push(InlineStyle::Italic);
                idx += 1;
            }
            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Strikethrough) => {
                style_stack.push(InlineStyle::Strikethrough);
                idx += 1;
            }
            Event::End(TagEnd::Strikethrough) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_string();
                let (link_text, new_idx) = parse_inline_until_end(events, idx + 1, TagEnd::Link);
                let inline = InlineContent::Link {
                    text: link_text,
                    url,
                };
                push_with_styles(&mut content, inline, &style_stack);
                idx = new_idx;
            }
            Event::SoftBreak => {
                content.push(InlineContent::SoftBreak);
                idx += 1;
            }
            Event::HardBreak => {
                content.push(InlineContent::HardBreak);
                idx += 1;
            }
            // Stop at block-level events or end tags
            _ => {
                break;
            }
        }
    }

    (content, idx)
}

fn parse_inline_until_end(
    events: &[Event<'_>],
    start: usize,
    end_tag: TagEnd,
) -> (Vec<InlineContent>, usize) {
    let mut content = Vec::new();
    let mut idx = start;
    let mut style_stack: Vec<InlineStyle> = Vec::new();

    while idx < events.len() {
        match &events[idx] {
            Event::End(tag) if *tag == end_tag => {
                return (content, idx + 1);
            }
            Event::Text(text) => {
                // Auto-detect URLs in text and convert to links
                for inline in autolink_text(text) {
                    push_with_styles(&mut content, inline, &style_stack);
                }
                idx += 1;
            }
            Event::Code(code) => {
                let inline = InlineContent::Code(code.to_string());
                push_with_styles(&mut content, inline, &style_stack);
                idx += 1;
            }
            Event::Start(Tag::Strong) => {
                style_stack.push(InlineStyle::Bold);
                idx += 1;
            }
            Event::End(TagEnd::Strong) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Emphasis) => {
                style_stack.push(InlineStyle::Italic);
                idx += 1;
            }
            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Strikethrough) => {
                style_stack.push(InlineStyle::Strikethrough);
                idx += 1;
            }
            Event::End(TagEnd::Strikethrough) => {
                style_stack.pop();
                idx += 1;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_string();
                let (link_text, new_idx) = parse_inline_until_end(events, idx + 1, TagEnd::Link);
                let inline = InlineContent::Link {
                    text: link_text,
                    url,
                };
                push_with_styles(&mut content, inline, &style_stack);
                idx = new_idx;
            }
            Event::SoftBreak => {
                content.push(InlineContent::SoftBreak);
                idx += 1;
            }
            Event::HardBreak => {
                content.push(InlineContent::HardBreak);
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    (content, idx)
}

#[derive(Clone, Copy)]
enum InlineStyle {
    Bold,
    Italic,
    Strikethrough,
}

fn push_with_styles(content: &mut Vec<InlineContent>, item: InlineContent, styles: &[InlineStyle]) {
    let mut result = item;
    for style in styles.iter().rev() {
        result = match style {
            InlineStyle::Bold => InlineContent::Bold(vec![result]),
            InlineStyle::Italic => InlineContent::Italic(vec![result]),
            InlineStyle::Strikethrough => InlineContent::Strikethrough(vec![result]),
        };
    }
    content.push(result);
}

/// Convert text containing bare URLs into a mix of Text and Link nodes
fn autolink_text(text: &str) -> Vec<InlineContent> {
    let mut result = Vec::new();
    let mut last_end = 0;

    for mat in URL_REGEX.find_iter(text) {
        // Add text before the URL
        if mat.start() > last_end {
            result.push(InlineContent::Text(text[last_end..mat.start()].to_string()));
        }

        // Add the URL as a link
        let url = mat.as_str().to_string();
        result.push(InlineContent::Link {
            text: vec![InlineContent::Text(url.clone())],
            url,
        });

        last_end = mat.end();
    }

    // Add any remaining text after the last URL
    if last_end < text.len() {
        result.push(InlineContent::Text(text[last_end..].to_string()));
    }

    // If no URLs found, just return the original text
    if result.is_empty() {
        result.push(InlineContent::Text(text.to_string()));
    }

    result
}

fn collect_code_block(events: &[Event<'_>], start: usize) -> (String, usize) {
    let mut code = String::new();
    let mut idx = start;

    while idx < events.len() {
        match &events[idx] {
            Event::End(TagEnd::CodeBlock) => {
                return (code, idx + 1);
            }
            Event::Text(text) => {
                code.push_str(text);
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    (code, idx)
}

fn parse_block_until_end(
    events: &[Event<'_>],
    start: usize,
    end_tag: TagEnd,
) -> (Vec<MarkdownElement>, usize) {
    let mut idx = start;
    let mut nested_events = Vec::new();
    let mut depth = 1;

    while idx < events.len() {
        match &events[idx] {
            Event::End(tag) if *tag == end_tag => {
                depth -= 1;
                if depth == 0 {
                    return (parse_events(&nested_events), idx + 1);
                }
                nested_events.push(events[idx].clone());
            }
            Event::Start(Tag::BlockQuote(_)) if matches!(end_tag, TagEnd::BlockQuote(_)) => {
                depth += 1;
                nested_events.push(events[idx].clone());
            }
            _ => {
                nested_events.push(events[idx].clone());
            }
        }
        idx += 1;
    }

    (parse_events(&nested_events), idx)
}

fn parse_list_items(events: &[Event<'_>], start: usize, _ordered: bool) -> (Vec<ListItem>, usize) {
    let mut items = Vec::new();
    let mut idx = start;

    while idx < events.len() {
        match &events[idx] {
            Event::End(TagEnd::List(_)) => {
                return (items, idx + 1);
            }
            Event::Start(Tag::Item) => {
                let (item, new_idx) = parse_list_item(events, idx + 1);
                items.push(item);
                idx = new_idx;
            }
            _ => {
                idx += 1;
            }
        }
    }

    (items, idx)
}

fn parse_list_item(events: &[Event<'_>], start: usize) -> (ListItem, usize) {
    let mut idx = start;
    let mut nested_events = Vec::new();
    let mut checked = None;

    while idx < events.len() {
        match &events[idx] {
            Event::End(TagEnd::Item) => {
                let content = parse_events(&nested_events);
                return (ListItem { content, checked }, idx + 1);
            }
            Event::TaskListMarker(is_checked) => {
                checked = Some(*is_checked);
                idx += 1;
            }
            _ => {
                nested_events.push(events[idx].clone());
                idx += 1;
            }
        }
    }

    let content = parse_events(&nested_events);
    (ListItem { content, checked }, idx)
}

fn parse_table(events: &[Event<'_>], start: usize) -> (Vec<TableCell>, Vec<Vec<TableCell>>, usize) {
    let mut headers = Vec::new();
    let mut rows = Vec::new();
    let mut idx = start;
    let mut in_head = false;
    let mut current_row: Vec<TableCell> = Vec::new();

    while idx < events.len() {
        match &events[idx] {
            Event::End(TagEnd::Table) => {
                return (headers, rows, idx + 1);
            }
            Event::Start(Tag::TableHead) => {
                in_head = true;
                idx += 1;
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                idx += 1;
            }
            Event::Start(Tag::TableRow) => {
                current_row = Vec::new();
                idx += 1;
            }
            Event::End(TagEnd::TableRow) => {
                if in_head {
                    headers = std::mem::take(&mut current_row);
                } else {
                    rows.push(std::mem::take(&mut current_row));
                }
                idx += 1;
            }
            Event::Start(Tag::TableCell) => {
                let (content, new_idx) = parse_inline_until_end(events, idx + 1, TagEnd::TableCell);
                current_row.push(TableCell { content });
                idx = new_idx;
            }
            _ => {
                idx += 1;
            }
        }
    }

    (headers, rows, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autolink_detection() {
        let result = autolink_text("Check https://example.com here");
        assert_eq!(result.len(), 3);

        match &result[0] {
            InlineContent::Text(t) => assert_eq!(t, "Check "),
            _ => panic!("Expected Text"),
        }

        match &result[1] {
            InlineContent::Link { url, .. } => assert_eq!(url, "https://example.com"),
            _ => panic!("Expected Link"),
        }

        match &result[2] {
            InlineContent::Text(t) => assert_eq!(t, " here"),
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn test_autolink_in_paragraph() {
        let elements = parse("Visit https://support.anthropic.com for help.");
        assert_eq!(elements.len(), 1);

        if let MarkdownElement::Paragraph(content) = &elements[0] {
            // Should contain: Text("Visit "), Link, Text(" for help.")
            let has_link = content
                .iter()
                .any(|c| matches!(c, InlineContent::Link { .. }));
            assert!(has_link, "Paragraph should contain a link: {:?}", content);
        } else {
            panic!("Expected Paragraph");
        }
    }
}
