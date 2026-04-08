import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';
import { useColorScheme } from 'react-native';
import {
  createTheme,
  darkTheme,
  lightTheme,
  type Theme,
  type ColorScheme,
  type ResolvedScheme,
} from '@krusty/ui';

interface ThemeContextValue {
  theme: Theme;
  colorScheme: ColorScheme;
  setColorScheme: (scheme: ColorScheme) => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: darkTheme,
  colorScheme: 'system',
  setColorScheme: () => {},
});

export function ThemeProvider({ children }: { children: ReactNode }) {
  const systemScheme = useColorScheme();
  const [colorScheme, setColorScheme] = useState<ColorScheme>('dark');
  const resolvedSystemScheme: ResolvedScheme =
    systemScheme === 'light' ? 'light' : 'dark';

  const resolved: ResolvedScheme =
    colorScheme === 'system'
      ? resolvedSystemScheme
      : colorScheme;

  const theme = resolved === 'dark' ? darkTheme : lightTheme;

  return (
    <ThemeContext.Provider value={{ theme, colorScheme, setColorScheme }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useThemeContext() {
  return useContext(ThemeContext);
}
