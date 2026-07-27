const GENERIC_THREAD_TITLES = new Set([
  "new chat",
  "new session",
  "session",
  "untitled",
]);

const TIMESTAMP_SESSION_TITLE =
  /^session\s+\d{4}-\d{2}-\d{2}(?:[ t]\d{1,2}:\d{2}(?::\d{2})?)?$/i;

export function displayThreadTitle(title?: string | null): string {
  const normalized = title?.trim() ?? "";
  if (!normalized) {
    return "";
  }
  if (GENERIC_THREAD_TITLES.has(normalized.toLowerCase())) {
    return "";
  }
  if (TIMESTAMP_SESSION_TITLE.test(normalized)) {
    return "";
  }
  return normalized;
}
