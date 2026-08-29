import { forwardRef } from 'react';
import { View } from 'react-native';
import {
  requireNativeViewManager,
  requireOptionalNativeModule,
} from 'expo-modules-core';
import type {
  MitsuroLiquidGlassViewProps,
  MitsuroLiquidGlassViewRef,
} from './MitsuroLiquidGlass.types';

const nativeModule = requireOptionalNativeModule('MitsuroLiquidGlass');
const NativeMitsuroLiquidGlassView = nativeModule
  ? requireNativeViewManager<MitsuroLiquidGlassViewProps>(
      'MitsuroLiquidGlass',
      'MitsuroLiquidGlassView',
    )
  : null;

/**
 * The native view is deliberately non-interactive. Wrap this exported
 * component with Reanimated.createAnimatedComponent to drive numeric progress
 * props from animatedProps without moving the native glass host itself.
 */
const MitsuroLiquidGlassView = forwardRef<
  MitsuroLiquidGlassViewRef,
  MitsuroLiquidGlassViewProps
>(function MitsuroLiquidGlassView(props, ref) {
  if (!NativeMitsuroLiquidGlassView) {
    return (
      <View
        ref={ref}
        style={props.style}
        testID={props.testID}
        pointerEvents="none"
        accessible={false}
      />
    );
  }

  return (
    <NativeMitsuroLiquidGlassView
      {...props}
      ref={ref}
      pointerEvents="none"
      accessible={false}
    />
  );
});

export default MitsuroLiquidGlassView;
