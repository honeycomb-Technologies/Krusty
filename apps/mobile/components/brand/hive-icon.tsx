import Svg, { Path } from "react-native-svg";

import { MITSURO_CELL_PATH } from "./mitsuro-mark";

interface HiveIconProps {
  size?: number;
  color?: string;
  strokeWidth?: number;
}

const CELLS = [
  "translate(385 16) scale(.42) translate(512 512) rotate(30) scale(.88) translate(-512 -512)",
  "translate(172 371) scale(.42) translate(512 512) rotate(30) scale(.88) translate(-512 -512)",
  "translate(598 371) scale(.42) translate(512 512) rotate(30) scale(.88) translate(-512 -512)",
] as const;

export function HiveIcon({
  size = 24,
  color = "currentColor",
  strokeWidth = 2,
}: HiveIconProps) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 1200 850"
      fill="none"
      accessibilityRole="image"
      accessibilityLabel="Hive"
    >
      {CELLS.map((transform) => (
        <Path
          key={transform}
          d={MITSURO_CELL_PATH}
          transform={transform}
          stroke={color}
          strokeWidth={strokeWidth * 58}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      ))}
    </Svg>
  );
}
