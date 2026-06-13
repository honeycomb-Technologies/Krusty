use gpui::{div, AnyElement, IntoElement, ParentElement as _, Styled as _};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::design::theme;

pub fn markdown_view(content: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(render_markdown_blocks(content))
}

fn render_markdown_blocks(content: &str) -> Vec<AnyElement> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(content, options);

    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_body = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_paragraph(&mut blocks, &mut paragraph);
                in_code = true;
                code_body.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(code_block(&code_lang, &code_body));
                in_code = false;
                code_lang.clear();
                code_body.clear();
            }
            Event::Code(text) if in_code => code_body.push_str(&text),
            Event::Text(text) if in_code => code_body.push_str(&text),
            Event::Text(text) => paragraph.push_str(&text),
            Event::SoftBreak | Event::HardBreak => paragraph.push('\n'),
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => flush_paragraph(&mut blocks, &mut paragraph),
            _ => {}
        }
    }

    flush_paragraph(&mut blocks, &mut paragraph);
    if blocks.is_empty() {
        blocks.push(
            div()
                .text_sm()
                .text_color(theme::text_muted())
                .child("…")
                .into_any_element(),
        );
    }
    blocks
}

fn flush_paragraph(blocks: &mut Vec<AnyElement>, paragraph: &mut String) {
    let trimmed = paragraph.trim();
    if trimmed.is_empty() {
        paragraph.clear();
        return;
    }
    blocks.push(
        div()
            .text_sm()
            .text_color(theme::text())
            .child(trimmed.to_owned())
            .into_any_element(),
    );
    paragraph.clear();
}

fn code_block(lang: &str, body: &str) -> AnyElement {
    div()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
        .flex()
        .flex_col()
        .child(
            div()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(theme::hairline())
                .text_xs()
                .text_color(theme::text_muted())
                .child(if lang.is_empty() {
                    "code".to_owned()
                } else {
                    lang.to_owned()
                }),
        )
        .child(
            div()
                .p_2()
                .text_xs()
                .text_color(theme::text())
                .child(body.trim_end().to_owned()),
        )
        .into_any_element()
}
