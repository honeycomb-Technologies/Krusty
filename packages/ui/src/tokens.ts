// iOS 26 Liquid Glass inspired design tokens

export const colors = {
  // Core
  background: '#0b1119',
  foreground: '#fafafa',
  primary: '#fafafa',
  primaryMuted: 'rgba(250, 250, 250, 0.7)',

  // Messages
  userMessage: '#ff6b35',
  userMessageBg: 'rgba(255, 107, 53, 0.15)',
  aiMessage: 'rgba(255, 255, 255, 0.04)',
  thinking: '#ff6b35',
  thinkingBg: 'rgba(255, 107, 53, 0.1)',

  // Status
  success: '#22c55e',
  error: '#ef4444',
  warning: '#f59e0b',
  info: '#3b82f6',

  // Surfaces
  card: 'rgba(255, 255, 255, 0.04)',
  muted: '#27272a',
  mutedForeground: '#a1a1aa',
  border: 'rgba(255, 255, 255, 0.08)',
  destructive: '#7f1d1d',
  destructiveForeground: '#fca5a5',

  // Glass
  glass: {
    background: 'rgba(255, 255, 255, 0.06)',
    backgroundElevated: 'rgba(255, 255, 255, 0.10)',
    backgroundPressed: 'rgba(255, 255, 255, 0.14)',
    border: 'rgba(255, 255, 255, 0.12)',
    borderLight: 'rgba(255, 255, 255, 0.20)',
    blur: 20,
    blurIntense: 40,
  },

  // Light mode overrides
  light: {
    background: '#ffffff',
    foreground: '#0a0a0a',
    primary: '#0a0a0a',
    card: 'rgba(0, 0, 0, 0.03)',
    muted: '#f4f4f5',
    mutedForeground: '#71717a',
    border: 'rgba(0, 0, 0, 0.08)',
    userMessageBg: 'rgba(255, 107, 53, 0.10)',
    aiMessage: 'rgba(0, 0, 0, 0.03)',
    glass: {
      background: 'rgba(255, 255, 255, 0.60)',
      backgroundElevated: 'rgba(255, 255, 255, 0.75)',
      backgroundPressed: 'rgba(255, 255, 255, 0.85)',
      border: 'rgba(0, 0, 0, 0.08)',
      borderLight: 'rgba(0, 0, 0, 0.12)',
    },
  },
} as const;

export const spacing = {
  xxs: 2,
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 24,
  xxl: 32,
  xxxl: 48,
} as const;

export const radii = {
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 22,
  xxl: 28,
  pill: 999,
} as const;

export const fontSizes = {
  xs: 11,
  sm: 13,
  base: 15,
  md: 17,
  lg: 20,
  xl: 24,
  xxl: 28,
  hero: 34,
} as const;

export const fontWeights = {
  regular: '400' as const,
  medium: '500' as const,
  semibold: '600' as const,
  bold: '700' as const,
};

export const fonts = {
  sans: undefined, // System default (SF Pro on iOS)
  mono: 'JetBrainsMono',
} as const;

export const shadows = {
  sm: {
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.15,
    shadowRadius: 2,
    elevation: 2,
  },
  md: {
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.2,
    shadowRadius: 8,
    elevation: 4,
  },
  lg: {
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.25,
    shadowRadius: 16,
    elevation: 8,
  },
  glow: (color: string) => ({
    shadowColor: color,
    shadowOffset: { width: 0, height: 0 },
    shadowOpacity: 0.4,
    shadowRadius: 12,
    elevation: 0,
  }),
} as const;

export const animation = {
  fast: 150,
  normal: 250,
  slow: 400,
  spring: {
    damping: 20,
    stiffness: 300,
    mass: 0.8,
  },
} as const;
