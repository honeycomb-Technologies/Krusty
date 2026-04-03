import { View, type ViewStyle, type StyleProp } from 'react-native';

interface LinearGradientProps {
  colors: readonly string[];
  start?: { x: number; y: number };
  end?: { x: number; y: number };
  style?: StyleProp<ViewStyle>;
  children?: React.ReactNode;
}

export function LinearGradient({ colors, start, end, style, children }: LinearGradientProps) {
  const s = start ?? { x: 0, y: 0 };
  const e = end ?? { x: 1, y: 0 };
  const angle = Math.round(Math.atan2(e.y - s.y, e.x - s.x) * (180 / Math.PI) + 90);
  const gradient = `linear-gradient(${angle}deg, ${colors.join(', ')})`;

  return (
    <View style={[{ backgroundImage: gradient } as any, style]}>
      {children}
    </View>
  );
}
