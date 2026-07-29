import { useEffect, type ComponentProps } from "react";
import Svg, { G, Path } from "react-native-svg";
import Animated, {
  Easing,
  interpolate,
  useAnimatedProps,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

export const MITSURO_CELL_PATH =
  "M365 132h294c47 0 76 17 99 58l151 269c20 36 20 70 0 106L758 834c-23 41-52 58-99 58H365c-47 0-76-17-99-58L115 565c-20-36-20-70 0-106l151-269c23-41 52-58 99-58Z";

const CELL_SEGMENTS = [
  "M365 132h294c47 0 76 17 99 58",
  "M758 190l151 269c20 36 20 70 0 106",
  "M909 565L758 834c-23 41-52 58-99 58",
  "M659 892H365c-47 0-76-17-99-58",
  "M266 834L115 565c-20-36-20-70 0-106",
  "M115 459l151-269c23-41 52-58 99-58",
] as const;

const AnimatedPath = Animated.createAnimatedComponent(Path);
const TRACE_LENGTH = 420;
type SvgStyle = ComponentProps<typeof Svg>["style"];

export interface MitsuroMarkProps {
  size?: number;
  color?: string;
  fill?: string;
  strokeWidth?: number;
  style?: SvgStyle;
  testID?: string;
}

export function MitsuroMark({
  size = 24,
  color = "#c5c1c8",
  fill = "none",
  strokeWidth = 52,
  style,
  testID,
}: MitsuroMarkProps) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      fill="none"
      style={style}
      testID={testID}
      accessibilityRole="image"
      accessibilityLabel="Mitsuro"
    >
      <G transform="translate(512 512) rotate(30) scale(.88) translate(-512 -512)">
        <Path
          d={MITSURO_CELL_PATH}
          fill={fill}
          stroke={color}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </G>
    </Svg>
  );
}

export interface MitsuroTraceMarkProps extends MitsuroMarkProps {
  duration?: number;
  playKey?: string | number;
}

export function MitsuroTraceMark({
  size = 88,
  color = "#c5c1c8",
  fill = "rgba(197, 193, 200, 0.08)",
  strokeWidth = 42,
  duration = 680,
  playKey = 0,
  style,
  testID,
}: MitsuroTraceMarkProps) {
  const reducedMotion = useReducedMotion();
  const progress = useSharedValue(reducedMotion ? 1 : 0);

  useEffect(() => {
    if (reducedMotion) {
      progress.value = 1;
      return;
    }

    progress.value = 0;
    progress.value = withTiming(1, {
      duration,
      easing: Easing.bezier(0.22, 1, 0.36, 1),
    });
  }, [duration, playKey, progress, reducedMotion]);

  const traceProps = useAnimatedProps(() => ({
    strokeDashoffset: TRACE_LENGTH * (1 - progress.value),
    opacity: interpolate(progress.value, [0, 0.08, 1], [0, 1, 1], "clamp"),
  }));
  const fillProps = useAnimatedProps(() => ({
    fillOpacity: interpolate(progress.value, [0.68, 1], [0, 1], "clamp"),
  }));

  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      fill="none"
      style={style}
      testID={testID}
      accessibilityRole="image"
      accessibilityLabel="Mitsuro"
    >
      <G transform="translate(512 512) rotate(30) scale(.88) translate(-512 -512)">
        <AnimatedPath
          d={MITSURO_CELL_PATH}
          fill={fill}
          stroke="none"
          animatedProps={fillProps}
        />
        {CELL_SEGMENTS.map((segment) => (
          <AnimatedPath
            key={segment}
            d={segment}
            fill="none"
            stroke={color}
            strokeWidth={strokeWidth}
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeDasharray={`${TRACE_LENGTH} ${TRACE_LENGTH}`}
            animatedProps={traceProps}
          />
        ))}
      </G>
    </Svg>
  );
}
