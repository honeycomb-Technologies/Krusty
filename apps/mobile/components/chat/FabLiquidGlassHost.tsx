import { memo } from 'react';
import { type ColorValue, StyleSheet } from 'react-native';
import Animated, {
  type SharedValue,
  useAnimatedProps,
} from 'react-native-reanimated';

import {
  MitsuroLiquidGlassView,
  type MitsuroLiquidGlassViewProps,
} from '../../modules/mitsuro-liquid-glass';
import {
  FAB_GAP,
  FAB_PILL,
  FAB_RADIUS,
  FAB_STEP,
} from './fabGooey';
import type { FabGlassMotion } from './fabGlassMotion';

const ROOT_HORIZONTAL_PADDING = 10;
const PROVIDER_DOCK_RIGHT_RESERVE = FAB_PILL + FAB_GAP + ROOT_HORIZONTAL_PADDING;
const PROVIDER_CONTENT_PADDING = 34;
const PROVIDER_FIRST_CENTER = 62;

const AnimatedLiquidGlassView = Animated.createAnimatedComponent(
  MitsuroLiquidGlassView,
);

interface FabLiquidGlassHostProps {
  active: boolean;
  motion: FabGlassMotion;
  modelProgress: SharedValue<number>;
  accordionOpen: boolean;
  verticalCount: number;
  attachmentOpen: boolean;
  attachmentSourceIndex: number;
  providerOpen: boolean;
  providerCount: number;
  providerSourceIndex: number;
  modelOpen: boolean;
  bandWidth: number;
  viewportHeight: number;
  inputRowBottom: number;
  overlayBottom: number;
  modelPopoverHeight: number;
  tintColor: ColorValue;
  colorScheme: 'dark' | 'light';
}

/**
 * One stationary native host owns every moving iOS glass surface. React Native
 * keeps the controls, icons, gestures and accessibility above it. The host is
 * inset by the same 10pt left edge as the composer, which also clips provider
 * glass exactly where the scrolling rail begins while retaining the Agent
 * source square on the right.
 */
function FabLiquidGlassHostComponent({
  active,
  motion,
  modelProgress,
  accordionOpen,
  verticalCount,
  attachmentOpen,
  attachmentSourceIndex,
  providerOpen,
  providerCount,
  providerSourceIndex,
  modelOpen,
  bandWidth,
  viewportHeight,
  inputRowBottom,
  overlayBottom,
  modelPopoverHeight,
  tintColor,
  colorScheme,
}: FabLiquidGlassHostProps) {
  const hostWidth = Math.max(0, bandWidth - ROOT_HORIZONTAL_PADDING);
  const rootX = hostWidth - ROOT_HORIZONTAL_PADDING - FAB_PILL / 2;
  const rootY = viewportHeight - inputRowBottom - FAB_PILL / 2;
  const modelWidth = Math.max(
    0,
    bandWidth - ROOT_HORIZONTAL_PADDING * 2 - FAB_PILL - FAB_GAP,
  );
  const modelX = modelWidth / 2;
  const modelY = viewportHeight - overlayBottom - modelPopoverHeight / 2;
  const providerDockWidth = Math.max(
    0,
    bandWidth - PROVIDER_DOCK_RIGHT_RESERVE,
  );
  const estimatedProviderContentWidth =
    PROVIDER_CONTENT_PADDING + providerCount * FAB_STEP;
  const providerLeadingSlack = Math.max(
    0,
    providerDockWidth - estimatedProviderContentWidth,
  );
  const providerSourceY = rootY - FAB_STEP * (providerSourceIndex + 1);

  const animatedProps = useAnimatedProps<MitsuroLiquidGlassViewProps>(() => {
    const providerVisualIndex = (branchIndex: number): number =>
      providerCount - branchIndex - 1;
    const providerTargetX = (branchIndex: number): number => {
      const visualIndex = providerVisualIndex(branchIndex);
      if (visualIndex < 0) return rootX;
      const reorderOffset = motion.providerReorderX[branchIndex].value;
      const dragOffset = motion.providerDragIndex.value === visualIndex
        ? motion.providerDragX.value
        : reorderOffset;
      return providerLeadingSlack
        + PROVIDER_FIRST_CENTER
        + visualIndex * FAB_STEP
        - motion.providerScrollX.value
        + dragOffset;
    };
    const providerDragAmount = (branchIndex: number): number =>
      motion.providerDragging[branchIndex].value;
    const providerTargetY = (branchIndex: number): number =>
      providerSourceY
      - motion.providerEditProgress.value * 2
      - providerDragAmount(branchIndex) * 4;
    const providerTargetScale = (branchIndex: number): number =>
      1
      + motion.providerEditProgress.value * 0.02
      + providerDragAmount(branchIndex) * 0.05;
    const providerTargetRotation = (branchIndex: number): number => {
      const visualIndex = providerVisualIndex(branchIndex);
      if (visualIndex < 0) return 0;
      return motion.providerEditProgress.value
        * (visualIndex % 2 === 0 ? -1.2 : 1.2);
    };

    return {
      p0: motion.pillProgresses[0].value,
      p1: motion.pillProgresses[1].value,
      p2: motion.pillProgresses[2].value,
      p3: motion.pillProgresses[3].value,
      p4: motion.pillProgresses[4].value,
      p5: motion.pillProgresses[5].value,
      attachmentP0: motion.attachmentProgresses[0].value,
      attachmentP1: motion.attachmentProgresses[1].value,
      attachmentP2: motion.attachmentProgresses[2].value,
      q0: motion.providerProgresses[0].value,
      q1: motion.providerProgresses[1].value,
      q2: motion.providerProgresses[2].value,
      q3: motion.providerProgresses[3].value,
      q4: motion.providerProgresses[4].value,
      q5: motion.providerProgresses[5].value,
      providerX0: providerTargetX(0),
      providerX1: providerTargetX(1),
      providerX2: providerTargetX(2),
      providerX3: providerTargetX(3),
      providerX4: providerTargetX(4),
      providerX5: providerTargetX(5),
      providerY0: providerTargetY(0),
      providerY1: providerTargetY(1),
      providerY2: providerTargetY(2),
      providerY3: providerTargetY(3),
      providerY4: providerTargetY(4),
      providerY5: providerTargetY(5),
      providerScale0: providerTargetScale(0),
      providerScale1: providerTargetScale(1),
      providerScale2: providerTargetScale(2),
      providerScale3: providerTargetScale(3),
      providerScale4: providerTargetScale(4),
      providerScale5: providerTargetScale(5),
      providerRotation0: providerTargetRotation(0),
      providerRotation1: providerTargetRotation(1),
      providerRotation2: providerTargetRotation(2),
      providerRotation3: providerTargetRotation(3),
      providerRotation4: providerTargetRotation(4),
      providerRotation5: providerTargetRotation(5),
      providerViewportClip: motion.providerViewportClip.value,
      modelProgress: modelProgress.value,
    };
  }, [
    modelProgress,
    motion,
    providerCount,
    providerLeadingSlack,
    providerSourceY,
    rootX,
  ]);

  if (!active || hostWidth <= 0 || viewportHeight <= 0) return null;

  return (
    <AnimatedLiquidGlassView
      animatedProps={animatedProps}
      pointerEvents="none"
      mode="global"
      open={accordionOpen}
      count={verticalCount}
      rootX={rootX}
      rootY={rootY}
      rootWidth={FAB_PILL}
      rootHeight={FAB_PILL}
      rootCornerRadius={FAB_RADIUS}
      showComposer={false}
      verticalStep={FAB_STEP}
      p0={0}
      p1={0}
      p2={0}
      p3={0}
      p4={0}
      p5={0}
      attachmentOpen={attachmentOpen}
      attachmentCount={3}
      attachmentP0={0}
      attachmentP1={0}
      attachmentP2={0}
      attachmentSourceIndex={attachmentSourceIndex}
      attachmentStep={FAB_STEP}
      providerOpen={providerOpen}
      providerCount={providerCount}
      q0={0}
      q1={0}
      q2={0}
      q3={0}
      q4={0}
      q5={0}
      providerSourceIndex={providerSourceIndex}
      providerStep={FAB_STEP}
      providerViewportClip={0}
      modelOpen={modelOpen}
      modelProgress={0}
      modelSourceIndex={providerSourceIndex}
      modelX={modelX}
      modelY={modelY}
      modelWidth={modelWidth}
      modelHeight={modelPopoverHeight}
      modelCornerRadius={FAB_RADIUS}
      effectSpacing={8}
      tintColor={tintColor}
      colorScheme={colorScheme}
      style={[
        styles.host,
        {
          height: viewportHeight,
          width: hostWidth,
        },
      ]}
    />
  );
}

const styles = StyleSheet.create({
  host: {
    position: 'absolute',
    left: ROOT_HORIZONTAL_PADDING,
    bottom: 0,
    overflow: 'hidden',
    zIndex: 0,
  },
});

export const FabLiquidGlassHost = memo(FabLiquidGlassHostComponent);
