import { colors, spacing, radii, fontSizes, fontWeights, fonts, shadows, animation } from './tokens';

export type ColorScheme = 'dark' | 'light' | 'system';
export type ResolvedScheme = 'dark' | 'light';

export interface Theme {
  scheme: ResolvedScheme;
  colors: {
    background: string;
    foreground: string;
    primary: string;
    primaryMuted: string;
    userMessage: string;
    userMessageBg: string;
    aiMessage: string;
    thinking: string;
    thinkingBg: string;
    success: string;
    error: string;
    warning: string;
    info: string;
    card: string;
    muted: string;
    mutedForeground: string;
    border: string;
    destructive: string;
    destructiveForeground: string;
    glass: {
      background: string;
      backgroundElevated: string;
      backgroundPressed: string;
      border: string;
      borderLight: string;
    };
    glassBlur: number;
    glassBlurIntense: number;
  };
  spacing: typeof spacing;
  radii: typeof radii;
  fontSizes: typeof fontSizes;
  fontWeights: typeof fontWeights;
  fonts: typeof fonts;
  shadows: typeof shadows;
  animation: typeof animation;
}

export function createTheme(scheme: ResolvedScheme): Theme {
  const isDark = scheme === 'dark';

  return {
    scheme,
    colors: {
      background: isDark ? colors.background : colors.light.background,
      foreground: isDark ? colors.foreground : colors.light.foreground,
      primary: isDark ? colors.primary : colors.light.primary,
      primaryMuted: isDark ? colors.primaryMuted : 'rgba(36, 35, 38, 0.70)',
      userMessage: colors.userMessage,
      userMessageBg: isDark ? colors.userMessageBg : colors.light.userMessageBg,
      aiMessage: isDark ? colors.aiMessage : colors.light.aiMessage,
      thinking: colors.thinking,
      thinkingBg: colors.thinkingBg,
      success: colors.success,
      error: colors.error,
      warning: colors.warning,
      info: colors.info,
      card: isDark ? colors.card : colors.light.card,
      muted: isDark ? colors.muted : colors.light.muted,
      mutedForeground: isDark ? colors.mutedForeground : colors.light.mutedForeground,
      border: isDark ? colors.border : colors.light.border,
      destructive: colors.destructive,
      destructiveForeground: colors.destructiveForeground,
      glass: isDark ? {
        background: colors.glass.background,
        backgroundElevated: colors.glass.backgroundElevated,
        backgroundPressed: colors.glass.backgroundPressed,
        border: colors.glass.border,
        borderLight: colors.glass.borderLight,
      } : colors.light.glass,
      glassBlur: colors.glass.blur,
      glassBlurIntense: colors.glass.blurIntense,
    },
    spacing,
    radii,
    fontSizes,
    fontWeights,
    fonts,
    shadows,
    animation,
  };
}

export const darkTheme = createTheme('dark');
export const lightTheme = createTheme('light');
