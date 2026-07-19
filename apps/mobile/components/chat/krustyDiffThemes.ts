import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";
import type { ThemeRegistration } from "@shikijs/core";

/**
 * One syntax palette for every Krusty surface. The surrounding diff chrome is
 * still driven by the active app theme, while token colors remain identical in
 * Pierre (web/desktop) and Shiki (iOS/Android).
 */
export const krustyDarkDiffTheme: ThemeRegistration = {
  ...githubDark,
  name: "krusty-dark",
};

export const krustyLightDiffTheme: ThemeRegistration = {
  ...githubLight,
  name: "krusty-light",
};

export const KRUSTY_DIFF_THEME_NAMES = {
  dark: "krusty-dark",
  light: "krusty-light",
} as const;
