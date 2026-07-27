import Svg, { Path } from "react-native-svg";

interface MakoSharkIconProps {
  size?: number;
  color?: string;
  strokeWidth?: number;
}

/**
 * Lucide Shark icon. Kept local until the installed react-native package
 * includes the upstream glyph supplied by the product shell.
 */
export function MakoSharkIcon({
  size = 24,
  color = "currentColor",
  strokeWidth = 2,
}: MakoSharkIconProps) {
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={color}
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <Path d="M3.6 15a9.07 9.07 0 0 0 11.7 5.3S19 22 22 22c0 0-1-3-3-4.5 1.1-1.5 1.9-3.3 2-5.3l-8 4.6a1.94 1.94 0 1 1-2-3.4l6-3.5s5-2.8 5-6.8c0-.6-.4-1-1-1h-9c-1.8 0-3.4.5-4.8 1.5C5.7 2.5 3.9 2 2 2c0 0 1.4 2.1 2.3 4.5A10.63 10.63 0 0 0 3.1 13" />
      <Path d="M13.8 7 13 6" />
      <Path d="M21.12 6h-3.5c-1.1 0-2.8.5-3.82 1L9 9.8C3 11 2 15 2 15h4" />
    </Svg>
  );
}
