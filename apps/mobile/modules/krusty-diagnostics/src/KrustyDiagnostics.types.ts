export type NativeMetricKitPayload = NativeMetricKitV1Payload | NativeMetricKitV2Payload;

export interface NativeMetricKitPayloadBase {
  id: string;
  kind: 'metric' | 'diagnostic';
  receivedAtMs: number;
}

export interface NativeMetricKitV1Payload extends NativeMetricKitPayloadBase {
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

export interface NativeMetricKitV2Payload extends NativeMetricKitPayloadBase {
  kind: 'diagnostic';
  summarySchemaVersion: 2;
  periodStartMs: number;
  periodEndMs: number;
  diagnostics: NativeMetricKitDiagnostic[];
}

export type NativeMetricKitDiagnosticType =
  | 'crash'
  | 'hang'
  | 'cpu_exception'
  | 'disk_write_exception';

export interface NativeMetricKitDiagnostic {
  type: NativeMetricKitDiagnosticType;
  appVersion: string;
  buildVersion: string;
  architecture: string;
  stacks: NativeMetricKitStack[];
}

export interface NativeMetricKitStack {
  fingerprintSha256: string;
  threadAttributed: boolean;
  frames: NativeMetricKitFrame[];
}

export interface NativeMetricKitFrame {
  binaryUuid: string;
  binaryName: string;
  /** Unsigned 64-bit offset encoded in canonical base-10 to avoid JS precision loss. */
  offset: string;
  sampleCount: number;
}
