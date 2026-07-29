import { NativeModule, requireOptionalNativeModule } from 'expo';
import type { NativeMetricKitPayload } from './KrustyDiagnostics.types';

declare class KrustyDiagnosticsModule extends NativeModule<{}> {
  isMetricKitAvailable(): boolean;
  getBuildNumber(): string | null;
  beginPerformanceSpan(spanId: number, name: string): void;
  endPerformanceSpan(spanId: number, name: string): void;
  listMetricKitPayloads(): Promise<NativeMetricKitPayload[]>;
  acknowledgeMetricKitPayloads(ids: string[]): Promise<void>;
}

export default requireOptionalNativeModule<KrustyDiagnosticsModule>('KrustyDiagnostics');
