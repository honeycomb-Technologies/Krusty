import { forwardRef } from 'react';
import { View } from 'react-native';
import type {
  MitsuroLiquidGlassViewProps,
  MitsuroLiquidGlassViewRef,
} from './MitsuroLiquidGlass.types';

/** Transparent fallback for Android, web, Expo Go and unlinked native builds. */
const MitsuroLiquidGlassView = forwardRef<
  MitsuroLiquidGlassViewRef,
  MitsuroLiquidGlassViewProps
>(function MitsuroLiquidGlassView({ style, testID }, ref) {
  return (
    <View
      ref={ref}
      style={style}
      testID={testID}
      pointerEvents="none"
      accessible={false}
    />
  );
});

export default MitsuroLiquidGlassView;
