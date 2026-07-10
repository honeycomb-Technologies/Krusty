import { View, type ViewStyle, type StyleProp } from 'react-native';

interface LinearGradientProps {
  colors: readonly string[];
  locations?: readonly number[];
  start?: { x: number; y: number };
  end?: { x: number; y: number };
  style?: StyleProp<ViewStyle>;
  children?: React.ReactNode;
}

export function LinearGradient({ colors, locations, start, end, style, children }: LinearGradientProps) {
  const s = start ?? { x: 0.5, y: 0 };
  const e = end ?? { x: 0.5, y: 1 };
  const angle = Math.round(Math.atan2(e.y - s.y, e.x - s.x) * (180 / Math.PI) + 90);
  const stops = colors.map((color, index) => {
    const location = locations?.[index];
    return typeof location === 'number'
      ? `${color} ${Math.max(0, Math.min(100, location * 100))}%`
      : color;
  });
  const gradient = `linear-gradient(${angle}deg, ${stops.join(', ')})`;

  return (
    <View style={[{ backgroundImage: gradient } as any, style]}>
      {children}
    </View>
  );
}

export type { LinearGradientProps };
