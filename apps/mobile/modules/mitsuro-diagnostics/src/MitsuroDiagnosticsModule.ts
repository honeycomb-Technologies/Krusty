import { NativeModule, requireOptionalNativeModule } from 'expo';
import type { NativeMetricKitPayload } from './MitsuroDiagnostics.types';
import { LEGACY_DIAGNOSTICS_MODULE_NAME } from './compatibility';

declare class MitsuroDiagnosticsModule extends NativeModule<{}> {
  isMetricKitAvailable(): boolean;
  getBuildNumber(): string | null;
  beginPerformanceSpan(spanId: number, name: string): void;
  endPerformanceSpan(spanId: number, name: string): void;
  recordJsHotPathProbe(payload: string): void;
  listMetricKitPayloads(): Promise<NativeMetricKitPayload[]>;
  acknowledgeMetricKitPayloads(ids: string[]): Promise<void>;
}

export default (
  requireOptionalNativeModule<MitsuroDiagnosticsModule>('MitsuroDiagnostics')
  ?? requireOptionalNativeModule<MitsuroDiagnosticsModule>(LEGACY_DIAGNOSTICS_MODULE_NAME)
);
