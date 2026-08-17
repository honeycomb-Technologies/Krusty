import { type ReactNode } from 'react';
import { View, StyleSheet, type StyleProp, type ViewStyle } from 'react-native';
import { useThemeContext } from '../../hooks/useTheme';

interface GlassCardProps {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  elevated?: boolean;
  intensity?: number;
  compact?: boolean;
}

export function GlassCard({
  children,
  style,
  elevated,
  compact = false,
}: GlassCardProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const radius = compact ? 8 : theme.radii.xl;

  return (
    <View
      style={[
        styles.wrapper,
        {
          borderRadius: radius,
          backgroundColor: elevated ? t.surfaceElevated : t.surface,
          borderColor: t.border,
        },
        style,
      ]}
    >
      <View style={[styles.content, compact && styles.compactContent]}>{children}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    position: 'relative',
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
  },
  content: {
    padding: 16,
  },
  compactContent: {
    padding: 0,
  },
});
