export const STATE_POLL_INTERVAL = 3000;
export const STATE_POLL_MAX_BACKOFF = 30_000;
export const STATE_POLL_MAX_FAILURES = 5;
export const STATE_POLL_DEGRADED_AFTER = 2;
export const STATE_POLL_DEGRADED_MESSAGE =
  'Connection to session status is unstable. Retrying…';
export const PRESENCE_HEARTBEAT_INTERVAL = 10_000;
export const MAX_QUEUED_MESSAGES = 50;
export const MAX_MESSAGE_CONTENT_LENGTH = 500_000;
export const PRESENCE_CLIENT_STORAGE_KEY = 'krusty:presence-client-id';
export const SUPPORTED_IMAGE_MIME_TYPES = [
  'image/jpeg',
  'image/png',
  'image/gif',
  'image/webp',
] as const;
