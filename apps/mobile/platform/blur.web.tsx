import { View, type ViewStyle, type StyleProp } from 'react-native';

interface BlurViewProps {
  intensity?: number;
  tint?: string;
  style?: StyleProp<ViewStyle>;
  children?: React.ReactNode;
}

export function BlurView({ intensity = 20, style, children }: BlurViewProps) {
  return (
    <View
      style={[
        { backdropFilter: `blur(${Math.round(intensity * 0.4)}px)`, WebkitBackdropFilter: `blur(${Math.round(intensity * 0.4)}px)` } as any,
        style,
      ]}
    >
      {children}
    </View>
  );
}

export type { BlurViewProps };
