export const STATE_POLL_INTERVAL = 3000;
export const STATE_POLL_MAX_BACKOFF = 30_000;
export const STATE_POLL_MAX_FAILURES = 5;
export const STATE_POLL_DEGRADED_AFTER = 2;
export const STATE_POLL_DEGRADED_MESSAGE =
  'Connection to session status is unstable. Retrying…';
export const PRESENCE_HEARTBEAT_INTERVAL = 10_000;
export const MAX_QUEUED_MESSAGES = 50;
export const MAX_MESSAGE_CONTENT_LENGTH = 500_000;
/** Hard UI retention for live streamed assistant text. */
export const MAX_LIVE_MESSAGE_CONTENT_LENGTH = 120_000;
/** Hard UI retention for live thinking content. */
export const MAX_LIVE_THINKING_CONTENT_LENGTH = 40_000;
/** Hard UI retention for each tool output string. */
export const MAX_LIVE_TOOL_OUTPUT_LENGTH = 80_000;
/** Max messages retained in each warm session cache entry. */
export const MAX_CACHED_SESSION_MESSAGES = 80;
/** Max lastKnownServerState entries retained per mode store. */
export const MAX_LAST_KNOWN_SERVER_STATE = 32;
export const PRESENCE_CLIENT_STORAGE_KEY = 'krusty:presence-client-id';
export const SUPPORTED_IMAGE_MIME_TYPES = [
  'image/jpeg',
  'image/png',
  'image/gif',
  'image/webp',
] as const;
