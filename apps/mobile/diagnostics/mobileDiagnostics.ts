import type {
  DiagnosticEventType,
  DiagnosticFields,
  DiagnosticSnapshot,
  MobileDiagnosticRecorder,
} from '@krusty/state';

let activeRecorder: MobileDiagnosticRecorder | null = null;

export function installMobileDiagnosticRecorder(
  recorder: MobileDiagnosticRecorder | null,
): void {
  activeRecorder = recorder;
}

export function recordMobileDiagnostic(
  type: DiagnosticEventType,
  fields: DiagnosticFields = {},
): void {
  activeRecorder?.record(type, fields);
}

export function recordRequestDiagnostic(
  name: string,
  outcome: 'start' | 'complete' | 'cancel' | 'error',
  durationMs?: number,
): void {
  recordMobileDiagnostic('request', { name, outcome, durationMs });
}

export function recordWebViewDiagnostic(
  surface: 'browser' | 'terminal' | 'html_preview',
  outcome: 'mount' | 'ready' | 'terminate' | 'reload' | 'error' | 'unmount',
  code?: string,
): void {
  recordMobileDiagnostic('webview', { surface, outcome, code });
}

export function recordLiveActivityDiagnostic(
  outcome: 'start' | 'update' | 'end' | 'error',
  durationMs?: number,
): void {
  recordMobileDiagnostic('live_activity', { outcome, durationMs });
}

export function getMobileDiagnosticSnapshot(): DiagnosticSnapshot | null {
  return activeRecorder?.snapshot() ?? null;
}
