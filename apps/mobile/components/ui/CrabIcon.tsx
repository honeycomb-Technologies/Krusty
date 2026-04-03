import Svg, { Path } from 'react-native-svg';

interface CrabIconProps {
  size?: number;
  color?: string;
}

export function CrabIcon({ size = 24, color = '#e6a700' }: CrabIconProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      {/* Body shell */}
      <Path
        d="M5.5 12C5.5 9 8 7 12 7s6.5 2 6.5 5c0 2.5-2 4.5-6.5 4.5S5.5 14.5 5.5 12z"
        stroke={color}
        strokeWidth={1.5}
      />
      {/* Left claw arm */}
      <Path
        d="M5.5 11C4 10.5 2.5 9.5 2 8.5"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Left claw pincer top */}
      <Path
        d="M2 8.5C1.2 7.8 1 7 1.5 6.5s1.5 0 2 .8"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Left claw pincer bottom */}
      <Path
        d="M2 8.5C1.5 9.2 1.8 10 2.5 10.2s1.2-.5 1-1.2"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Right claw arm */}
      <Path
        d="M18.5 11c1.5-.5 3-1.5 3.5-2.5"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Right claw pincer top */}
      <Path
        d="M22 8.5c.8-.7 1-1.5.5-2s-1.5 0-2 .8"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Right claw pincer bottom */}
      <Path
        d="M22 8.5c.5.7.2 1.5-.5 1.7s-1.2-.5-1-1.2"
        stroke={color}
        strokeWidth={1.5}
        strokeLinecap="round"
      />
      {/* Eyes */}
      <Path d="M9.5 7.5V5.5" stroke={color} strokeWidth={1.5} strokeLinecap="round" />
      <Path d="M14.5 7.5V5.5" stroke={color} strokeWidth={1.5} strokeLinecap="round" />
      <Path d="M9.5 5a.8.8 0 1 1 0-1.6.8.8 0 0 1 0 1.6z" fill={color} />
      <Path d="M14.5 5a.8.8 0 1 1 0-1.6.8.8 0 0 1 0 1.6z" fill={color} />
      {/* Legs - left */}
      <Path d="M6.5 13.5L4 16" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
      <Path d="M6 15L3.5 17" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
      <Path d="M7 16.5L5.5 18.5" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
      {/* Legs - right */}
      <Path d="M17.5 13.5L20 16" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
      <Path d="M18 15L20.5 17" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
      <Path d="M17 16.5L18.5 18.5" stroke={color} strokeWidth={1.3} strokeLinecap="round" />
    </Svg>
  );
}
