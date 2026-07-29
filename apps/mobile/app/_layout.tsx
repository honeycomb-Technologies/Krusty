import { useEffect } from 'react';
import { StatusBar } from 'expo-status-bar';
import { Stack, useRouter, useSegments } from 'expo-router';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { LogBox, StyleSheet } from 'react-native';
import { ThemeProvider, useThemeContext } from '../hooks/useTheme';
import { ConnectionProvider, useConnection } from '../hooks/useConnection';
import { StoresProvider } from '../hooks/useStores';
import { useDeepLink } from '../hooks/useDeepLink';
import { SplashProvider, useSplashState } from '../hooks/useSplashState';
import { SplashOverlay } from '../components/splash/SplashOverlay';
import { NotificationProvider } from '../hooks/useNotifications';
import { configureKrustyPerformance } from '@krusty/state';
import { MobileDiagnosticsProvider } from '../diagnostics/MobileDiagnosticsProvider';

const BOOT_BACKGROUND = '#0e0e11';

configureKrustyPerformance(
  __DEV__ || process.env.EXPO_PUBLIC_KRUSTY_PERFORMANCE === '1',
);

LogBox.ignoreLogs([
  'Invalid DOM property `%s`. Did you mean `%s`? transform-origin transformOrigin',
  'Invalid DOM property `transform-origin`. Did you mean `transformOrigin`?',
]);

const globalWithKrustyLogFilter = globalThis as typeof globalThis & {
  __krustySvgWarningFilterInstalled?: boolean;
};

if (!globalWithKrustyLogFilter.__krustySvgWarningFilterInstalled) {
  globalWithKrustyLogFilter.__krustySvgWarningFilterInstalled = true;
  const originalConsoleError = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    const message = args.map(String).join(' ');
    if (
      message.includes('Invalid DOM property') &&
      message.includes('transform-origin') &&
      message.includes('transformOrigin')
    ) {
      return;
    }
    originalConsoleError(...args);
  };
}

function RootNavigator() {
  const { theme } = useThemeContext();
  const { client, isConfigured } = useConnection();
  useDeepLink();
  const router = useRouter();
  const segments = useSegments();

  useEffect(() => {
    const inOnboarding = segments[0] === 'onboarding';
    const inNavigationPreview = segments[0] === 'navigation-preview';

    if (!isConfigured && !inOnboarding && !inNavigationPreview) {
      router.replace('/onboarding');
    } else if (isConfigured && inOnboarding) {
      router.replace('/(tabs)');
    }
  }, [isConfigured, segments]);

  const content = (
    <>
      <StatusBar style={theme.scheme === 'dark' ? 'light' : 'dark'} />
      <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: theme.colors.background },
          animation: 'fade',
        }}
      >
        <Stack.Screen name="(tabs)" />
        <Stack.Screen name="onboarding" />
      </Stack>
    </>
  );

  return (
    <NotificationProvider>
      <StoresProvider client={client}>{content}</StoresProvider>
    </NotificationProvider>
  );
}

function SplashWrapper({ children }: { children: React.ReactNode }) {
  const { markSplashDone } = useSplashState();

  return (
    <SplashOverlay onComplete={markSplashDone}>
      {children}
    </SplashOverlay>
  );
}

export default function RootLayout() {
  return (
    <GestureHandlerRootView style={styles.root}>
      <SafeAreaProvider>
        <SplashProvider>
          <SplashWrapper>
            <ThemeProvider>
              <ConnectionProvider>
                <MobileDiagnosticsProvider>
                  <RootNavigator />
                </MobileDiagnosticsProvider>
              </ConnectionProvider>
            </ThemeProvider>
          </SplashWrapper>
        </SplashProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: BOOT_BACKGROUND },
});
