import { type ReactNode } from 'react';
import { View, StyleSheet, type StyleProp, type ViewStyle } from 'react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { AdaptiveMaterial } from './AdaptiveMaterial';

interface GlassCardProps {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  elevated?: boolean;
  intensity?: number;
  compact?: boolean;
}

export function GlassCard({ children, style, elevated, intensity, compact = false }: GlassCardProps) {
  const { theme } = useThemeContext();
  const g = theme.colors.glass;

  const radius = compact ? 8 : theme.radii.xl;

  return (
    <View style={[styles.wrapper, { borderRadius: radius }, style]}>
      <AdaptiveMaterial
        borderRadius={radius}
        blurIntensity={intensity}
        tone={elevated ? "elevated" : "regular"}
      />
      <View
        style={[
          StyleSheet.absoluteFill,
          styles.ignorePointerEvents,
          {
            borderRadius: radius,
            borderWidth: StyleSheet.hairlineWidth,
            borderColor: g.border,
          },
        ]}
      />
      <View style={[styles.content, compact && styles.compactContent]}>{children}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    position: 'relative',
    overflow: 'hidden',
  },
  content: {
    padding: 16,
  },
  compactContent: {
    padding: 0,
  },
  ignorePointerEvents: {
    pointerEvents: 'none',
  },
});
