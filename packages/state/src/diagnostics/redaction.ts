import type { DiagnosticFields } from './types';

const MAX_LABEL_LENGTH = 64;
const SAFE_LABEL = /^[a-zA-Z0-9_.:()\- >]+$/;
const SENSITIVE_LABEL = /(?:authorization|bearer|token|secret|prompt|message|content|output|command|cookie|header)/i;
const KNOWN_LABELS = new Set([
  'active', 'background', 'inactive', 'unknown',
  'start', 'complete', 'cancel', 'error', 'mount', 'ready', 'terminate', 'reload', 'unmount',
  'connecting', 'connected', 'disconnected',
  'chat', 'code', 'hive', 'browser', 'terminal', 'html_preview',
  'diagnostics.ready', 'native.payloads', 'stress.start', 'stress.stop', 'stress.expired',
  'pending.recovered', 'js.drift', 'js.longtask', 'interaction', 'mode.change',
  'app.launch', 'new_chat.shell', 'new_chat.session_bind', 'session.open',
  'stream.connect', 'stream.first_event', 'stream.flush', 'stream.finish',
  'session.fetch_decode', 'session.snapshot_transform', 'session.snapshot_publish',
  'session.cache_compact',
  'session.snapshot_max_slice', 'session.snapshot_yields',
  'transcript.visible_messages', 'transcript.visible_render_parts',
  'transcript.visible_tools', 'transcript.visible_markdown_characters',
  'transcript.derive', 'transcript.visual_plan', 'transcript.first_paint',
  'mode.switch', 'toolbox.open', 'diagnostics.persist', 'diagnostics.upload',
  'server.connection',
  'api.health', 'api.sessions', 'api.sessions.catalog', 'api.sessions.detail',
  'api.sessions.state', 'api.sessions.workflow', 'api.sessions.presence',
  'api.sessions.action', 'api.sessions.directories',
  'api.models', 'api.credentials', 'api.auth',
  'api.mcp', 'api.skills', 'api.ports', 'api.hive', 'api.git', 'api.files',
  'api.notifications', 'api.mobile_diagnostics', 'api.stream', 'api.other',
  'http.2xx', 'http.3xx', 'http.4xx', 'http.5xx', 'http.unknown',
  'network.error', 'request.abort', 'decode.error',
  'live_activity.update', 'stream_connections', 'state_polling', 'presence_heartbeats',
  'session_requests', 'toolbox_requests', 'live_activity_updates',
  'root', '(tabs)', '(tabs)>index', '(tabs)>sessions', '(tabs)>settings',
  'settings', 'onboarding', 'navigation-preview', 'dynamic',
  'chat->chat', 'chat->code', 'chat->hive', 'code->chat', 'code->code', 'code->hive',
  'hive->chat', 'hive->code', 'hive->hive',
]);

/**
 * Diagnostics intentionally accept no arbitrary metadata object. Labels are
 * short identifiers only; prompts, messages, paths, URLs, headers, tokens,
 * terminal output, and file contents have no representable field.
 */
export function sanitizeDiagnosticFields(fields: DiagnosticFields): DiagnosticFields {
  const safe: DiagnosticFields = {};
  safe.name = sanitizeLabel(fields.name);
  safe.surface = sanitizeLabel(fields.surface);
  safe.state = sanitizeLabel(fields.state);
  safe.outcome = sanitizeLabel(fields.outcome);
  safe.code = sanitizeLabel(fields.code);
  if (Number.isFinite(fields.durationMs)) {
    safe.durationMs = roundAndClamp(fields.durationMs!, 0, 60 * 60 * 1000);
  }
  if (Number.isFinite(fields.count)) {
    safe.count = roundAndClamp(fields.count!, 0, 1_000_000);
  }
  return Object.fromEntries(
    Object.entries(safe).filter(([, value]) => value !== undefined),
  ) as DiagnosticFields;
}

export function sanitizeLabel(value: string | undefined): string | undefined {
  if (!value) return undefined;
  const trimmed = value.trim().slice(0, MAX_LABEL_LENGTH);
  if (!trimmed || !SAFE_LABEL.test(trimmed) || SENSITIVE_LABEL.test(trimmed)) {
    return 'redacted';
  }
  return KNOWN_LABELS.has(trimmed) ? trimmed : `label_${stableHash(trimmed)}`;
}

function stableHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

function roundAndClamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value * 10) / 10));
}
