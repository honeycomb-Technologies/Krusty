export type DiagnosticMode = 'baseline' | 'stress';

export type DiagnosticEventType =
  | 'app'
  | 'mode'
  | 'navigation'
  | 'app_state'
  | 'heartbeat'
  | 'longtask'
  | 'event_timing'
  | 'performance'
  | 'resource'
  | 'request'
  | 'webview'
  | 'live_activity'
  | 'diagnostic';

export interface DiagnosticFields {
  name?: string;
  surface?: string;
  state?: string;
  outcome?: string;
  code?: string;
  durationMs?: number;
  count?: number;
}

export interface DiagnosticEvent {
  id: string;
  sequence: number;
  type: DiagnosticEventType;
  atMs: number;
  mode: DiagnosticMode;
  fields: DiagnosticFields;
}

export interface DiagnosticBatch {
  schemaVersion: 1;
  installationId: string;
  runId: string;
  runStartedAtMs: number;
  createdAtMs: number;
  events: DiagnosticEvent[];
}

export interface DiagnosticSnapshot {
  mode: DiagnosticMode;
  installationId: string;
  runId: string;
  runStartedAtMs: number;
  eventCount: number;
  approximateBytes: number;
  events: DiagnosticEvent[];
}

export interface DiagnosticUploadMetadata {
  appVersion: string;
  buildNumber: string;
  platform: 'ios' | 'android' | 'web';
  osVersion: string;
  deviceClass: string;
  captureLevel: DiagnosticMode;
  completed?: boolean;
  endedAtMs?: number | null;
}

export interface DiagnosticUploadBatch {
  run: {
    id: string;
    installation_id: string;
    app_version: string;
    build_number: string;
    platform: 'ios' | 'android' | 'web';
    os_version: string;
    device_class: string;
    capture_level: DiagnosticMode;
    started_at_ms: number;
    ended_at_ms: number | null;
    dropped_event_count: number;
  };
  events: Array<{
    sequence: number;
    occurred_at_ms: number;
    monotonic_ms: number;
    category: string;
    name: string;
    duration_ms: number | null;
    severity: 'info' | 'warning' | 'error';
    attributes: Record<string, string | number | boolean>;
  }>;
  native_payloads: Array<{
    payload_id: string;
    kind: 'metric' | 'diagnostic';
    received_at_ms: number;
    payload_json: string;
  }>;
  completed: boolean;
}

export interface DiagnosticNativePayload {
  id: string;
  kind: 'metric' | 'diagnostic';
  receivedAtMs: number;
  summarySchemaVersion: 1;
  sourcePayloadBytes: number;
  hasApplicationLaunchMetrics: boolean;
  hasApplicationResponsivenessMetrics: boolean;
  hasMemoryMetrics: boolean;
  hasCpuMetrics: boolean;
  hasDiskIoMetrics: boolean;
  hasDisplayMetrics: boolean;
  hasNetworkTransferMetrics: boolean;
  hasApplicationExitMetrics: boolean;
  hasCellularConditionMetrics: boolean;
  hasLocationActivityMetrics: boolean;
  hasAnimationMetrics: boolean;
  crashDiagnosticCount: number;
  hangDiagnosticCount: number;
  cpuExceptionDiagnosticCount: number;
  diskWriteExceptionDiagnosticCount: number;
}

export interface DiagnosticUploadClient {
  /** Authenticated transport implemented by KrustyClient once Honey supports it. */
  uploadMobileDiagnostics(batch: DiagnosticUploadBatch): Promise<void>;
}
