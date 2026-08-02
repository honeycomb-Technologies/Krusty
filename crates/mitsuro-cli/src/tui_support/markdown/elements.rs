//! Markdown element types

/// Block-level markdown elements
#[derive(Debug, Clone)]
pub enum MarkdownElement {
    /// Regular paragraph with inline content
    Paragraph(Vec<InlineContent>),
    /// Heading with level (1-6) and content
    Heading {
        level: u8,
        content: Vec<InlineContent>,
    },
    /// Fenced code block with optional language
    CodeBlock { lang: Option<String>, code: String },
    /// Block quote containing nested elements
    BlockQuote(Vec<MarkdownElement>),
    /// List (ordered or unordered)
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    /// Table with headers and rows
    Table {
        headers: Vec<TableCell>,
        rows: Vec<Vec<TableCell>>,
    },
    /// Horizontal rule / thematic break
    ThematicBreak,
}

/// A list item containing block elements
#[derive(Debug, Clone)]
pub struct ListItem {
    pub content: Vec<MarkdownElement>,
    pub checked: Option<bool>,
}

/// A table cell containing inline content
#[derive(Debug, Clone)]
pub struct TableCell {
    pub content: Vec<InlineContent>,
}

/// Inline markdown content (text formatting)
#[derive(Debug, Clone)]
pub enum InlineContent {
    /// Plain text
    Text(String),
    /// Bold text
    Bold(Vec<InlineContent>),
    /// Italic text
    Italic(Vec<InlineContent>),
    /// Inline code
    Code(String),
    /// Hyperlink with OSC 8 clickable link support
    Link {
        text: Vec<InlineContent>,
        url: String,
    },
    /// Strikethrough text
    Strikethrough(Vec<InlineContent>),
    /// Soft line break (space)
    SoftBreak,
    /// Hard line break (newline)
    HardBreak,
}
