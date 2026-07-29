// Mitsuro Graphite Brass design tokens.

export const colors = {
  // Core
  background: '#0e0e11',
  foreground: '#e8e5ea',
  primary: '#e8e5ea',
  primaryMuted: 'rgba(232, 229, 234, 0.70)',

  // Messages
  userMessage: '#7f7485',
  userMessageBg: 'rgba(127, 116, 133, 0.22)',
  aiMessage: 'rgba(232, 229, 234, 0.045)',
  thinking: '#b89a61',
  thinkingBg: 'rgba(184, 154, 97, 0.12)',

  // Status
  success: '#7f9a86',
  error: '#b06f73',
  warning: '#b89a61',
  info: '#7f8fa3',

  // Surfaces
  card: 'rgba(232, 229, 234, 0.045)',
  muted: '#25282c',
  mutedForeground: '#9e9ba0',
  border: 'rgba(232, 229, 234, 0.10)',
  destructive: '#5f3035',
  destructiveForeground: '#d9a4a7',

  // Glass
  glass: {
    background: 'rgba(232, 229, 234, 0.055)',
    backgroundElevated: 'rgba(232, 229, 234, 0.095)',
    backgroundPressed: 'rgba(232, 229, 234, 0.14)',
    border: 'rgba(232, 229, 234, 0.12)',
    borderLight: 'rgba(232, 229, 234, 0.20)',
    blur: 20,
    blurIntense: 40,
  },

  // Light mode overrides
  light: {
    background: '#e6e3de',
    foreground: '#242326',
    primary: '#242326',
    card: 'rgba(36, 35, 38, 0.045)',
    muted: '#d7d3cd',
    mutedForeground: '#716d72',
    border: 'rgba(36, 35, 38, 0.11)',
    userMessageBg: 'rgba(127, 116, 133, 0.16)',
    aiMessage: 'rgba(36, 35, 38, 0.04)',
    glass: {
      background: 'rgba(246, 243, 238, 0.62)',
      backgroundElevated: 'rgba(246, 243, 238, 0.78)',
      backgroundPressed: 'rgba(246, 243, 238, 0.88)',
      border: 'rgba(36, 35, 38, 0.10)',
      borderLight: 'rgba(36, 35, 38, 0.15)',
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
