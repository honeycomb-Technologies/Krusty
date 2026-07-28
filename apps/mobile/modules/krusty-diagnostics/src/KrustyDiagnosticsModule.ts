import { NativeModule, requireOptionalNativeModule } from 'expo';
import type { NativeMetricKitPayload } from './KrustyDiagnostics.types';

declare class KrustyDiagnosticsModule extends NativeModule<{}> {
  isMetricKitAvailable(): boolean;
  listMetricKitPayloads(): Promise<NativeMetricKitPayload[]>;
  acknowledgeMetricKitPayloads(ids: string[]): Promise<void>;
}

export default requireOptionalNativeModule<KrustyDiagnosticsModule>('KrustyDiagnostics');
