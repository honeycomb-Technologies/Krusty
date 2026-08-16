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

  return (
    <View style={[styles.wrapper, compact && styles.compactWrapper, style]}>
      {compact ? (
        <View
          style={[
            StyleSheet.absoluteFill,
            styles.ignorePointerEvents,
            { backgroundColor: theme.colors.background, borderRadius: 8 },
          ]}
        />
      ) : (
        <AdaptiveMaterial
          borderRadius={theme.radii.xl}
          blurIntensity={intensity}
          tone={elevated ? "elevated" : "regular"}
        />
      )}
      {compact ? null : (
        <View
          style={[
            StyleSheet.absoluteFill,
            styles.ignorePointerEvents,
            {
              borderRadius: theme.radii.xl,
              borderWidth: StyleSheet.hairlineWidth,
              borderColor: g.border,
            },
          ]}
        />
      )}
      <View style={[styles.content, compact && styles.compactContent]}>{children}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    position: 'relative',
    borderRadius: 22,
    overflow: 'hidden',
  },
  content: {
    padding: 16,
  },
  compactWrapper: {
    borderRadius: 8,
  },
  compactContent: {
    padding: 0,
  },
  ignorePointerEvents: {
    pointerEvents: 'none',
  },
});
