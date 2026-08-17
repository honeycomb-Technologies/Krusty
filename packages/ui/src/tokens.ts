// Mitsuro Graphite Brass design tokens.

export const colors = {
  // Core
  background: '#0e0e11',
  surface: '#141519',
  surfaceElevated: '#19181d',
  surfaceStrong: '#222027',
  surfaceOverlaySubtle: 'rgba(14, 14, 17, 0.60)',
  surfaceOverlay: 'rgba(14, 14, 17, 0.88)',
  surfaceOverlayElevated: 'rgba(14, 14, 17, 0.92)',
  surfaceOverlayStrong: 'rgba(14, 14, 17, 0.96)',
  codeSurface: '#141519',
  foreground: '#e8e5ea',
  primary: '#e8e5ea',
  primaryMuted: 'rgba(232, 229, 234, 0.70)',
  accent: '#75617e',
  accentPressed: '#66536f',
  accentSurface: 'rgba(117, 97, 126, 0.22)',
  onAccent: '#f7f4f8',
  // Pulse — activity accent used by the brand mark and running-line beam.
  pulse: '#9d73ff',

  // Messages
  userMessage: '#75617e',
  userMessageBg: 'rgba(117, 97, 126, 0.22)',
  aiMessage: 'rgba(232, 229, 234, 0.045)',
  thinking: '#9a82a5',
  thinkingBg: 'rgba(154, 130, 165, 0.14)',

  // Status
  success: '#7f9a86',
  error: '#b06f73',
  warning: '#b89a61',
  info: '#9a82a5',

  // Surfaces
  card: 'rgba(232, 229, 234, 0.045)',
  muted: '#25282c',
  mutedForeground: '#9e98a3',
  border: 'rgba(232, 229, 234, 0.10)',
  destructive: '#5f3035',
  destructiveForeground: '#d9a4a7',

  // Glass — dark platform clear, not a gray frost. Tint with the graphite
  // foundation so liquid glass stays on the same blend as the shell.
  glass: {
    background: 'rgba(14, 14, 17, 0.36)',
    backgroundElevated: 'rgba(14, 14, 17, 0.50)',
    backgroundPressed: 'rgba(14, 14, 17, 0.64)',
    border: 'rgba(154, 130, 165, 0.16)',
    borderLight: 'rgba(154, 130, 165, 0.26)',
    blur: 20,
    blurIntense: 40,
  },

  // Light mode overrides
  light: {
    background: '#e6e3de',
    surface: '#f0ede8',
    surfaceElevated: '#f6f3ee',
    surfaceStrong: '#d7d3cd',
    surfaceOverlaySubtle: 'rgba(246, 243, 238, 0.60)',
    surfaceOverlay: 'rgba(246, 243, 238, 0.88)',
    surfaceOverlayElevated: 'rgba(246, 243, 238, 0.92)',
    surfaceOverlayStrong: 'rgba(246, 243, 238, 0.96)',
    codeSurface: '#f3f0eb',
    foreground: '#242326',
    primary: '#242326',
    card: 'rgba(36, 35, 38, 0.045)',
    muted: '#d7d3cd',
    mutedForeground: '#716d72',
    border: 'rgba(36, 35, 38, 0.11)',
    userMessageBg: 'rgba(117, 97, 126, 0.16)',
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
