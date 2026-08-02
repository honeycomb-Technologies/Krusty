import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import type { ThemeRegistration } from "@shikijs/core";

/**
 * One syntax palette for every Mitsuro surface. The surrounding diff chrome is
 * still driven by the active app theme, while token colors remain identical in
 * Pierre (web/desktop) and Shiki (iOS/Android).
 */
export const mitsuroDarkDiffTheme: ThemeRegistration = {
  ...githubDark,
  name: "mitsuro-dark",
};

export const mitsuroLightDiffTheme: ThemeRegistration = {
  ...githubLight,
  name: "mitsuro-light",
};

export const MITSURO_DIFF_THEME_NAMES = {
  dark: "mitsuro-dark",
  light: "mitsuro-light",
} as const;
