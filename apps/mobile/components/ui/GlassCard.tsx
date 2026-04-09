import { type ReactNode } from 'react';
import { View, StyleSheet, type StyleProp, type ViewStyle } from 'react-native';
import { BlurView } from '../../platform/blur';
import { useThemeContext } from '../../hooks/useTheme';

interface GlassCardProps {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  elevated?: boolean;
  intensity?: number;
}

export function GlassCard({ children, style, elevated, intensity }: GlassCardProps) {
  const { theme } = useThemeContext();
  const g = theme.colors.glass;
  const bg = elevated ? g.backgroundElevated : g.background;

  return (
    <View style={[styles.wrapper, style]}>
      <BlurView
        intensity={intensity ?? theme.colors.glassBlur}
        tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
        style={StyleSheet.absoluteFill}
      />
      <View
        style={[
          StyleSheet.absoluteFill,
          {
            backgroundColor: bg,
            borderRadius: theme.radii.xl,
            borderWidth: StyleSheet.hairlineWidth,
            borderColor: g.border,
          },
        ]}
      />
      <View style={styles.content}>{children}</View>
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
});
