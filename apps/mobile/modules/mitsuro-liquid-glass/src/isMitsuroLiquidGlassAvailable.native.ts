import { requireOptionalNativeModule } from 'expo-modules-core';

interface MitsuroLiquidGlassNativeModule {
  isSupported?: boolean;
}

let cachedAvailability: boolean | undefined;

export function isMitsuroLiquidGlassAvailable(): boolean {
  if (cachedAvailability === undefined) {
    const nativeModule = requireOptionalNativeModule<MitsuroLiquidGlassNativeModule>(
      'MitsuroLiquidGlass',
    );
    cachedAvailability = Boolean(nativeModule?.isSupported);
  }
  return cachedAvailability;
}
