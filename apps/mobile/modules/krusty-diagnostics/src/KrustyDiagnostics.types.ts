export interface NativeMetricKitPayload {
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
