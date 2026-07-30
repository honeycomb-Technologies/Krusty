import {
  type ComponentType,
  type PropsWithChildren,
  memo,
  useEffect,
} from 'react';
import { Platform, StyleSheet, View, type StyleProp, type ViewStyle } from 'react-native';
import Animated, {
  cancelAnimation,
  Easing,
  useAnimatedProps,
  useSharedValue,
  withTiming,
} from 'react-native-reanimated';
import Svg, {
  Defs,
  FeGaussianBlur,
  Filter,
  LinearGradient as SvgLinearGradient,
  Path,
  Stop,
} from 'react-native-svg';

const RUN_LINE_HEIGHT = 3;
const RUN_LINE_BEAM_WIDTH = 370;
const RUN_LINE_SOFTNESS = 3;
const RUN_LINE_TAIL_SOFTNESS = 8;
const RUN_LINE_STROKE_WIDTH = 14;
const RUN_LINE_CORNER_RADIUS = 44;
const RUN_LINE_EDGE_INSET = 2;
const RUN_LINE_EXIT_EXTENSION = 18;
const RUN_LINE_EXIT_RISE = 10;
const RUN_LINE_DURATION_MS = 1700;

/** Shared with ChatBar for non-desktop corner climb. */
export const RUN_LINE_CORNER_CLIMB = 35;

const AnimatedPath = Animated.createAnimatedComponent(Path);
const SvgDefs = Defs as unknown as ComponentType<PropsWithChildren>;
const normalizedPathLength = { pathLength: 1 };
const RUN_LINE_TONAL_STOPS = {
  dark: [
    ['0', '#75617e', 0.64],
    ['0.14', '#9a82a5', 0.72],
    ['0.28', '#9f9da1', 0.68],
    ['0.42', '#b89a61', 0.74],
    ['0.5', '#d4bd89', 0.78],
    ['0.58', '#b89a61', 0.74],
    ['0.72', '#9f9da1', 0.68],
    ['0.86', '#9a82a5', 0.72],
    ['1', '#75617e', 0.64],
  ],
  light: [
    ['0', '#66536f', 0.58],
    ['0.14', '#75617e', 0.66],
    ['0.28', '#6b6f77', 0.62],
    ['0.42', '#806430', 0.68],
    ['0.5', '#97773f', 0.72],
    ['0.58', '#806430', 0.68],
    ['0.72', '#6b6f77', 0.62],
    ['0.86', '#75617e', 0.66],
    ['1', '#66536f', 0.58],
  ],
} as const;

export interface ChatBarRunningLineProps {
  active: boolean;
  width: number;
  cornerClimb: number;
  theme?: 'dark' | 'light';
  style?: StyleProp<ViewStyle>;
}

function ChatBarRunningLineComponent({
  active,
  width,
  cornerClimb,
  theme = 'dark',
  style,
}: ChatBarRunningLineProps) {
  const progress = useSharedValue(0);

  useEffect(() => {
    if (!active) {
      cancelAnimation(progress);
      progress.value = 0;
      return;
    }

    // Explicitly reset every pass instead of relying on Reanimated's repeat
    // wrapper. react-native-svg can retain the terminal animated dash props on
    // iOS, which makes subsequent passes look like only the end is looping.
    const restart = () => {
      cancelAnimation(progress);
      progress.value = 0;
      progress.value = withTiming(1, {
        duration: RUN_LINE_DURATION_MS,
        easing: Easing.linear,
      });
    };

    restart();
    const interval = setInterval(restart, RUN_LINE_DURATION_MS);
    return () => {
      clearInterval(interval);
      cancelAnimation(progress);
    };
  }, [active, progress]);

  // Skip path math while idle so stream toggles only pay geometry cost when active.
  const pathMetrics = active
    ? (() => {
        const safeWidth = Math.max(width, 1);
        const pathHeight = Math.max(
          RUN_LINE_HEIGHT,
          cornerClimb + RUN_LINE_EXIT_RISE + RUN_LINE_HEIGHT,
        );
        const strokeInset = RUN_LINE_HEIGHT / 2;
        const baseline = pathHeight - strokeInset;
        const curveTop = baseline - cornerClimb;
        const cornerRadius = Math.min(
          RUN_LINE_CORNER_RADIUS,
          Math.max(0, safeWidth / 2 - strokeInset),
        );
        const effectiveClimb = Math.min(cornerClimb, cornerRadius);
        const sideInset =
          cornerRadius > 0
            ? cornerRadius -
              Math.sqrt(
                Math.max(
                  0,
                  cornerRadius ** 2 - (cornerRadius - effectiveClimb) ** 2,
                ),
              )
            : strokeInset;
        const visibleSideInset = sideInset + RUN_LINE_EDGE_INSET;
        const visibleCornerRadius = cornerRadius + RUN_LINE_EDGE_INSET;
        const cornerArcLength =
          cornerRadius > 0
            ? cornerRadius *
              Math.acos((cornerRadius - effectiveClimb) / cornerRadius)
            : 0;
        const exitLength = Math.hypot(
          visibleSideInset + RUN_LINE_EXIT_EXTENSION,
          curveTop - strokeInset,
        );
        const pathLength = Math.max(
          1,
          safeWidth -
            cornerRadius * 2 +
            cornerArcLength * 2 +
            exitLength * 2,
        );
        const beamLength = Math.min(
          RUN_LINE_BEAM_WIDTH,
          Math.max(160, pathLength * 0.8),
          pathLength * 0.92,
        );
        // Web SVG honors pathLength normalization; react-native-svg on iOS does not.
        // Keep both renderers on the same visual geometry using their native units.
        const beamFraction = Math.min(0.92, Math.max(0.5, beamLength / pathLength));
        const usesNormalizedDashUnits = Platform.OS === 'web';
        const dashPathLengthProps = usesNormalizedDashUnits
          ? normalizedPathLength
          : {};
        const pathUnits = usesNormalizedDashUnits ? 1 : pathLength;
        const beamUnits = usesNormalizedDashUnits ? beamFraction : beamLength;
        const beamTravel = pathUnits + beamUnits;
        const edgePath =
          effectiveClimb > 0
            ? [
                `M ${-RUN_LINE_EXIT_EXTENSION} ${strokeInset}`,
                `L ${visibleSideInset} ${curveTop}`,
                `C ${visibleSideInset + (visibleCornerRadius - visibleSideInset) * 0.25}`,
                `${curveTop + effectiveClimb * 0.6}`,
                `${visibleCornerRadius - (visibleCornerRadius - visibleSideInset) * 0.22}`,
                `${baseline}`,
                `${visibleCornerRadius} ${baseline}`,
                `H ${safeWidth - visibleCornerRadius}`,
                `C ${safeWidth - visibleCornerRadius + (visibleCornerRadius - visibleSideInset) * 0.22}`,
                `${baseline}`,
                `${safeWidth - visibleSideInset - (visibleCornerRadius - visibleSideInset) * 0.25}`,
                `${curveTop + effectiveClimb * 0.6}`,
                `${safeWidth - visibleSideInset} ${curveTop}`,
                `L ${safeWidth + RUN_LINE_EXIT_EXTENSION} ${strokeInset}`,
              ].join(' ')
            : `M ${strokeInset} ${baseline} H ${safeWidth - strokeInset}`;
        return {
          safeWidth,
          pathHeight,
          beamUnits,
          beamTravel,
          edgePath,
          dashPathLengthProps,
        };
      })()
    : null;

  const beamUnits = pathMetrics?.beamUnits ?? 0;
  const beamTravel = pathMetrics?.beamTravel ?? 1;
  const washProps = useAnimatedProps(() => ({
    strokeDashoffset:
      beamUnits - progress.value * beamTravel,
  }));
  if (!active || !pathMetrics) return null;

  const {
    safeWidth,
    pathHeight,
    edgePath,
    dashPathLengthProps,
  } = pathMetrics;

  return (
    <View
      pointerEvents="none"
      style={[styles.runLineTrack, { height: pathHeight }, style]}
    >
      <Svg width={safeWidth} height={pathHeight}>
        <SvgDefs>
          <SvgLinearGradient
            id="mitsuroGraphiteBrassBeam"
            x1={0}
            y1={0}
            x2={safeWidth}
            y2={0}
            gradientUnits="userSpaceOnUse"
          >
            {RUN_LINE_TONAL_STOPS[theme].map(([offset, color, opacity]) => (
              <Stop
                key={offset}
                offset={offset}
                stopColor={color}
                stopOpacity={opacity}
              />
            ))}
          </SvgLinearGradient>
          <Filter
            id="mitsuroBeamSoftness"
            x="-20%"
            y="-160%"
            width="140%"
            height="420%"
          >
            <FeGaussianBlur
              stdDeviation={`${RUN_LINE_TAIL_SOFTNESS} ${RUN_LINE_SOFTNESS}`}
            />
          </Filter>
        </SvgDefs>
        <AnimatedPath
          animatedProps={washProps}
          d={edgePath}
          {...dashPathLengthProps}
          fill="none"
          stroke="url(#mitsuroGraphiteBrassBeam)"
          filter="url(#mitsuroBeamSoftness)"
          strokeOpacity={0.78}
          strokeWidth={RUN_LINE_STROKE_WIDTH}
          strokeLinecap="round"
          strokeDasharray={`${beamUnits} ${beamTravel - beamUnits}`}
        />
      </Svg>
    </View>
  );
}

const styles = StyleSheet.create({
  runLineTrack: {
    height: RUN_LINE_HEIGHT,
    overflow: 'visible',
  },
});

export const ChatBarRunningLine = memo(ChatBarRunningLineComponent);
