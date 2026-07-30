const BLOCK_MARKDOWN =
  /(^|\n)\s{0,3}(?:#{1,6}\s|>|[-+*]\s|\d+[.)]\s|```|~~~|\||(?:-{3,}|_{3,}|\*{3,})\s*(?=\n|$))/u;
const INLINE_MARKDOWN =
  /(?:\*\*|__|~~|`|\[[^\]\n]+\]\(|!\[|https?:\/\/|<[/a-zA-Z][^>]*>|\*[^*\n]+\*|_[^_\n]+_)/u;
const SETEXT_HEADING = /(^|\n)[^\n]+\n(?:=+|-+)\s*(?=\n|$)/u;

/**
 * Most chat prose does not need a Markdown parser. Keep the detector
 * conservative: uncertain syntax stays on the rich renderer so the steady UI
 * remains identical while plain paragraphs avoid a large native-node parse.
 */
export function canRenderAsPlainChatText(content: string): boolean {
  return (
    !BLOCK_MARKDOWN.test(content)
    && !INLINE_MARKDOWN.test(content)
    && !SETEXT_HEADING.test(content)
  );
}
