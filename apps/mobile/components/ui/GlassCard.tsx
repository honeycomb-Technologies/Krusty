import { type ReactNode } from 'react';
import { View, StyleSheet, type StyleProp, type ViewStyle } from 'react-native';
import { BlurView } from '../../platform/blur';
import { useThemeContext } from '../../hooks/useTheme';

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
  const bg = elevated ? g.backgroundElevated : g.background;

  return (
    <View style={[styles.wrapper, compact && styles.compactWrapper, style]}>
      {compact ? null : (
        <BlurView
          intensity={intensity ?? theme.colors.glassBlur}
          tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
          style={StyleSheet.absoluteFill}
        />
      )}
      <View
        style={[
          StyleSheet.absoluteFill,
          {
            backgroundColor: compact ? theme.colors.background : bg,
            borderRadius: compact ? 8 : theme.radii.xl,
            borderWidth: compact ? 0 : StyleSheet.hairlineWidth,
            borderColor: compact ? 'transparent' : g.border,
          },
        ]}
      />
      <View style={[styles.content, compact && styles.compactContent]}>{children}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: {
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
});
