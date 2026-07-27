const HTML_PREVIEW_LANGUAGES = new Set(["html", "htm"]);

export function isHtmlPreviewLanguage(sourceInfo: unknown): boolean {
  if (typeof sourceInfo !== "string") return false;
  const language = sourceInfo.trim().split(/\s+/, 1)[0]?.toLowerCase();
  return Boolean(language && HTML_PREVIEW_LANGUAGES.has(language));
}

export function hasClosedHtmlFence(markdown: string): boolean {
  const lines = markdown.split("\n");
  const openingIndex = lines.findIndex((line) =>
    /^ {0,3}(?:`{3,}|~{3,})/.test(line),
  );
  if (openingIndex < 0) return false;

  const opening = lines[openingIndex]?.match(/^ {0,3}(`{3,}|~{3,})/);
  if (!opening) return false;

  const marker = opening[1];
  const markerCharacter = marker[0];
  const closingPattern = new RegExp(
    `^ {0,3}${escapeRegExp(markerCharacter)}{${marker.length},}\\s*$`,
  );

  return lines.slice(openingIndex + 1).some((line) => closingPattern.test(line));
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
