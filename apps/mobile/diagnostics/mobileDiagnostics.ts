import type {
  DiagnosticEventType,
  DiagnosticFields,
  DiagnosticSnapshot,
  MobileDiagnosticRecorder,
} from '@krusty/state';
import KrustyDiagnosticsModule from '../modules/krusty-diagnostics';

let activeRecorder: MobileDiagnosticRecorder | null = null;

interface NativePerformanceGlobal {
  __KRUSTY_NATIVE_PERFORMANCE__?: {
    begin(spanId: number, name: string): void;
    end(spanId: number, name: string): void;
  };
}

export function installMobileDiagnosticRecorder(
  recorder: MobileDiagnosticRecorder | null,
): void {
  activeRecorder = recorder;
  const root = globalThis as typeof globalThis & NativePerformanceGlobal;
  const nativeModule = KrustyDiagnosticsModule;
  if (recorder && nativeModule) {
    root.__KRUSTY_NATIVE_PERFORMANCE__ = {
      begin: (spanId, name) => nativeModule.beginPerformanceSpan(spanId, name),
      end: (spanId, name) => nativeModule.endPerformanceSpan(spanId, name),
    };
  } else {
    delete root.__KRUSTY_NATIVE_PERFORMANCE__;
  }
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
  code?: string,
): void {
  if (!activeRecorder || name === 'api.mobile_diagnostics') return;
  const mode = activeRecorder.getMode();
  if (outcome === 'start' && mode !== 'stress') return;
  if (
    outcome === 'complete'
    && mode !== 'stress'
    && (durationMs === undefined || durationMs < 1_000)
  ) return;
  activeRecorder.record('request', { name, outcome, durationMs, code });
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
