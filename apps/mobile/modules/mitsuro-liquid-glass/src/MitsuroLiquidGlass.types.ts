import type { Ref } from 'react';
import type { ColorValue, View, ViewProps } from 'react-native';

export type MitsuroLiquidGlassMode =
  | 'global'
  | 'vertical'
  | 'horizontal'
  | 'panel';

export type MitsuroLiquidGlassColorScheme = 'auto' | 'light' | 'dark';

/**
 * Native-only glass artwork. React Native retains ownership of icons, hit
 * targets, accessibility and gestures; this view must be mounted behind them.
 * All coordinates are local to this view and x/y values denote shape centres.
 */
export interface MitsuroLiquidGlassViewProps extends ViewProps {
  ref?: Ref<View>;
  mode?: MitsuroLiquidGlassMode;

  /** Open/count provide a non-Reanimated fallback for the vertical branch. */
  open: boolean;
  count: number;

  /** Persistent Agent/root glass geometry. */
  rootX: number;
  rootY: number;
  rootWidth?: number;
  rootHeight?: number;
  rootCornerRadius?: number;

  /** Optional persistent composer glass geometry. */
  showComposer?: boolean;
  composerX?: number;
  composerY?: number;
  composerWidth?: number;
  composerHeight?: number;
  composerCornerRadius?: number;

  /** Vertical FAB destination spacing. Index 0 is nearest the Agent. */
  verticalStep?: number;
  p0?: number;
  p1?: number;
  p2?: number;
  p3?: number;
  p4?: number;
  p5?: number;

  /** Attachment row, emitted leftward from a vertical FAB. */
  attachmentOpen?: boolean;
  attachmentCount?: number;
  attachmentP0?: number;
  attachmentP1?: number;
  attachmentP2?: number;
  attachmentSourceIndex?: number;
  attachmentStep?: number;

  /** Provider row, emitted leftward from the model FAB. */
  providerOpen?: boolean;
  providerCount?: number;
  q0?: number;
  q1?: number;
  q2?: number;
  q3?: number;
  q4?: number;
  q5?: number;
  providerX0?: number;
  providerX1?: number;
  providerX2?: number;
  providerX3?: number;
  providerX4?: number;
  providerX5?: number;
  providerY0?: number;
  providerY1?: number;
  providerY2?: number;
  providerY3?: number;
  providerY4?: number;
  providerY5?: number;
  providerScale0?: number;
  providerScale1?: number;
  providerScale2?: number;
  providerScale3?: number;
  providerScale4?: number;
  providerScale5?: number;
  providerRotation0?: number;
  providerRotation1?: number;
  providerRotation2?: number;
  providerRotation3?: number;
  providerRotation4?: number;
  providerRotation5?: number;
  /** 0 keeps the source corridor open; 1 clips settled cells to the rail. */
  providerViewportClip?: number;
  providerScrollShift?: number;
  providerSourceIndex?: number;
  providerStep?: number;

  /** Model panel target. It morphs from modelSourceIndex into this rectangle. */
  modelOpen?: boolean;
  modelProgress?: number;
  modelSourceIndex?: number;
  modelX?: number;
  modelY?: number;
  modelWidth?: number;
  modelHeight?: number;
  modelCornerRadius?: number;

  /**
   * Distance at which native glass shapes begin merging. Keep this below the
   * settled 10pt gap. The native container enforces a 17-shape ceiling; profile
   * a physical iOS 26 device before increasing that budget.
   */
  effectSpacing?: number;
  tintColor?: ColorValue;
  colorScheme?: MitsuroLiquidGlassColorScheme;
}

export type MitsuroLiquidGlassViewRef = View;
