//! Bounded Markdown presentation for assistant transcript messages.
//!
//! This intentionally implements the compact subset used most often in model
//! output: headings, paragraphs, lists, quotes, rules, fenced code, emphasis,
//! inline code, and links. Parsing is allocation-bounded by the caller's text
//! cap and never executes or fetches linked content.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, relative, AnyElement, FontStyle, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, ParentElement as _, Styled as _, StyledText,
};

use crate::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RichText {
    text: String,
    marks: Vec<InlineMark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineMark {
    range: Range<usize>,
    kind: InlineKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineKind {
    Bold,
    Italic,
    Code,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MarkdownBlock {
    Paragraph(RichText),
    Heading(u8, RichText),
    Code { language: String, body: String },
    Quote(RichText),
    List { ordered: bool, items: Vec<RichText> },
    Rule,
}

pub(super) fn markdown_body(index: u64, body: &str) -> impl IntoElement {
    let blocks = parse_markdown(body);
    div()
        .id(("markdown-body", index))
        .flex()
        .flex_col()
        .w_full()
        .gap(px(8.0))
        .children(
            blocks
                .into_iter()
                .enumerate()
                .map(move |(block_index, block)| render_block(index, block_index as u64, block)),
        )
}

fn render_block(message_index: u64, block_index: u64, block: MarkdownBlock) -> AnyElement {
    let colors = theme::colors();
    let id = message_index.saturating_mul(10_000) + block_index;
    match block {
        MarkdownBlock::Paragraph(text) => div()
            .id(("md-paragraph", id))
            .w_full()
            .text_sm()
            .line_height(relative(1.48))
            .text_color(colors.text)
            .whitespace_normal()
            .child(styled_inline(text))
            .into_any_element(),
        MarkdownBlock::Heading(level, text) => div()
            .id(("md-heading", id))
            .w_full()
            .pt(if level <= 2 { px(4.0) } else { px(1.0) })
            .text_size(px(match level {
                1 => 20.0,
                2 => 17.0,
                _ => 15.0,
            }))
            .line_height(relative(1.3))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors.text)
            .child(styled_inline(text))
            .into_any_element(),
        MarkdownBlock::Code { language, body } => div()
            .id(("md-code", id))
            .w_full()
            .rounded(px(10.0))
            .border_1()
            .border_color(colors.border_subtle)
            .bg(colors.bg_elevated)
            .overflow_hidden()
            .flex()
            .flex_col()
            .when(!language.is_empty(), |this| {
                this.child(
                    div()
                        .px(px(11.0))
                        .py(px(5.0))
                        .bg(colors.bg_sidebar)
                        .border_b_1()
                        .border_color(colors.border_subtle)
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(language),
                )
            })
            .child(
                div()
                    .px(px(11.0))
                    .py(px(9.0))
                    .text_xs()
                    .line_height(relative(1.45))
                    .font_family("monospace")
                    .text_color(colors.text_secondary)
                    .whitespace_normal()
                    .child(body),
            )
            .into_any_element(),
        MarkdownBlock::Quote(text) => div()
            .id(("md-quote", id))
            .w_full()
            .border_l_2()
            .border_color(colors.border)
            .pl(px(11.0))
            .py(px(2.0))
            .text_sm()
            .line_height(relative(1.45))
            .text_color(colors.text_secondary)
            .child(styled_inline(text))
            .into_any_element(),
        MarkdownBlock::List { ordered, items } => div()
            .id(("md-list", id))
            .flex()
            .flex_col()
            .w_full()
            .gap(px(4.0))
            .children(
                items
                    .into_iter()
                    .enumerate()
                    .map(move |(item_index, item)| {
                        div()
                            .id(("md-list-item", id.saturating_mul(1_000) + item_index as u64))
                            .flex()
                            .items_start()
                            .gap(px(8.0))
                            .pl(px(2.0))
                            .child(
                                div()
                                    .min_w(px(18.0))
                                    .text_sm()
                                    .line_height(relative(1.48))
                                    .text_color(colors.text_tertiary)
                                    .child(if ordered {
                                        format!("{}.", item_index + 1)
                                    } else {
                                        "•".to_owned()
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .text_sm()
                                    .line_height(relative(1.48))
                                    .text_color(colors.text)
                                    .child(styled_inline(item)),
                            )
                            .into_any_element()
                    }),
            )
            .into_any_element(),
        MarkdownBlock::Rule => div()
            .id(("md-rule", id))
            .w_full()
            .h(px(1.0))
            .my(px(3.0))
            .bg(colors.border_subtle)
            .into_any_element(),
    }
}

fn styled_inline(text: RichText) -> StyledText {
    let colors = theme::colors();
    let highlights = text.marks.into_iter().map(|mark| {
        let style = match mark.kind {
            InlineKind::Bold => HighlightStyle {
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
            InlineKind::Italic => HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
            InlineKind::Code => HighlightStyle {
                color: Some(colors.text_secondary),
                background_color: Some(colors.bg_elevated),
                ..Default::default()
            },
            InlineKind::Link => HighlightStyle {
                color: Some(colors.accent),
                ..Default::default()
            },
        };
        (mark.range, style)
    });
    StyledText::new(text.text).with_highlights(highlights)
}

fn parse_markdown(input: &str) -> Vec<MarkdownBlock> {
    let lines: Vec<&str> = input.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if let Some((fence, language)) = fenced_start(trimmed) {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].trim_start().starts_with(fence) {
                body.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            blocks.push(MarkdownBlock::Code {
                language: language.to_owned(),
                body: body.join("\n"),
            });
            continue;
        }

        if let Some((level, text)) = heading(line) {
            blocks.push(MarkdownBlock::Heading(level, parse_inline(text)));
            index += 1;
            continue;
        }

        if is_rule(trimmed) {
            blocks.push(MarkdownBlock::Rule);
            index += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            let mut quote = Vec::new();
            while index < lines.len() {
                let current = lines[index].trim_start();
                let Some(rest) = current.strip_prefix('>') else {
                    break;
                };
                quote.push(rest.trim_start());
                index += 1;
            }
            blocks.push(MarkdownBlock::Quote(parse_inline(&quote.join(" "))));
            continue;
        }

        if let Some((ordered, first)) = list_item(line) {
            let mut items = vec![parse_inline(first)];
            index += 1;
            while index < lines.len() {
                match list_item(lines[index]) {
                    Some((next_ordered, text)) if next_ordered == ordered => {
                        items.push(parse_inline(text));
                        index += 1;
                    }
                    _ => break,
                }
            }
            blocks.push(MarkdownBlock::List { ordered, items });
            continue;
        }

        let mut paragraph = vec![trimmed];
        index += 1;
        while index < lines.len() {
            let next = lines[index].trim();
            if next.is_empty() || starts_block(lines[index]) {
                break;
            }
            paragraph.push(next);
            index += 1;
        }
        blocks.push(MarkdownBlock::Paragraph(parse_inline(&paragraph.join(" "))));
    }

    if blocks.is_empty() {
        blocks.push(MarkdownBlock::Paragraph(parse_inline(input)));
    }
    blocks
}

fn starts_block(line: &str) -> bool {
    let trimmed = line.trim();
    fenced_start(trimmed).is_some()
        || heading(line).is_some()
        || is_rule(trimmed)
        || trimmed.starts_with('>')
        || list_item(line).is_some()
}

fn fenced_start(line: &str) -> Option<(&'static str, &str)> {
    if let Some(rest) = line.strip_prefix("```") {
        Some(("```", rest.trim()))
    } else if let Some(rest) = line.strip_prefix("~~~") {
        Some(("~~~", rest.trim()))
    } else {
        None
    }
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let count = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&count) || !trimmed[count..].starts_with(' ') {
        return None;
    }
    Some((count as u8, trimmed[count..].trim_start()))
}

fn list_item(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(text) = trimmed.strip_prefix(prefix) {
            return Some((false, text));
        }
    }
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 {
        let rest = &trimmed[digit_count..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((true, text));
        }
    }
    None
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn parse_inline(input: &str) -> RichText {
    let mut text = String::with_capacity(input.len());
    let mut marks = Vec::new();
    let mut rest = input;

    while !rest.is_empty() {
        if let Some((kind, delimiter)) = inline_delimiter(rest) {
            let after_open = &rest[delimiter.len()..];
            if let Some(close) = after_open.find(delimiter) {
                let content = &after_open[..close];
                let start = text.len();
                text.push_str(content);
                marks.push(InlineMark {
                    range: start..text.len(),
                    kind,
                });
                rest = &after_open[close + delimiter.len()..];
                continue;
            }
        }
        if let Some(label) = rest.strip_prefix('[') {
            if let Some(label_end) = label.find("](") {
                let after = &label[label_end + 2..];
                if let Some(url_end) = after.find(')') {
                    let start = text.len();
                    text.push_str(&label[..label_end]);
                    marks.push(InlineMark {
                        range: start..text.len(),
                        kind: InlineKind::Link,
                    });
                    rest = &after[url_end + 1..];
                    continue;
                }
            }
        }
        let mut chars = rest.char_indices();
        let (_, ch) = chars.next().expect("non-empty inline remainder");
        text.push(ch);
        let next = chars.next().map(|(i, _)| i).unwrap_or(rest.len());
        rest = &rest[next..];
    }

    RichText { text, marks }
}

fn inline_delimiter(input: &str) -> Option<(InlineKind, &'static str)> {
    if input.starts_with("**") {
        Some((InlineKind::Bold, "**"))
    } else if input.starts_with("__") {
        Some((InlineKind::Bold, "__"))
    } else if input.starts_with('`') {
        Some((InlineKind::Code, "`"))
    } else if input.starts_with('*') {
        Some((InlineKind::Italic, "*"))
    } else if input.starts_with('_') {
        Some((InlineKind::Italic, "_"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_model_markdown_into_semantic_blocks() {
        let blocks = parse_markdown(
            "## Result\n\n- first\n- second\n\n```rust\nfn main() {}\n```\n\n> note",
        );
        assert!(matches!(blocks[0], MarkdownBlock::Heading(2, _)));
        assert!(matches!(
            blocks[1],
            MarkdownBlock::List { ordered: false, .. }
        ));
        assert!(
            matches!(blocks[2], MarkdownBlock::Code { ref language, .. } if language == "rust")
        );
        assert!(matches!(blocks[3], MarkdownBlock::Quote(_)));
    }

    #[test]
    fn inline_markup_is_removed_but_style_ranges_remain_utf8_safe() {
        let rich =
            parse_inline("Use **Mitsuro** with `cargo test` and [docs](https://example.com). ✓");
        assert_eq!(rich.text, "Use Mitsuro with cargo test and docs. ✓");
        assert_eq!(rich.marks.len(), 3);
        assert!(rich
            .marks
            .iter()
            .all(|mark| rich.text.is_char_boundary(mark.range.start)
                && rich.text.is_char_boundary(mark.range.end)));
    }
}
