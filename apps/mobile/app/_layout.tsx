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
import { configureMitsuroPerformance } from '@mitsuro/state';
import { MobileDiagnosticsProvider } from '../diagnostics/MobileDiagnosticsProvider';
import { installJsHotPathProbe } from '../diagnostics/jsHotPathProbe';

const BOOT_BACKGROUND = '#0e0e11';

installJsHotPathProbe();
configureMitsuroPerformance(
  __DEV__ ||
    process.env.EXPO_PUBLIC_MITSURO_PERFORMANCE === '1' ||
    process.env.EXPO_PUBLIC_KRUSTY_PERFORMANCE === '1',
);

LogBox.ignoreLogs([
  'Invalid DOM property `%s`. Did you mean `%s`? transform-origin transformOrigin',
  'Invalid DOM property `transform-origin`. Did you mean `transformOrigin`?',
]);

const globalWithMitsuroLogFilter = globalThis as typeof globalThis & {
  __mitsuroSvgWarningFilterInstalled?: boolean;
};

if (!globalWithMitsuroLogFilter.__mitsuroSvgWarningFilterInstalled) {
  globalWithMitsuroLogFilter.__mitsuroSvgWarningFilterInstalled = true;
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
  const { client, isConfigured, hasLoadedConnection } = useConnection();
  useDeepLink();
  const router = useRouter();
  const segments = useSegments();

  useEffect(() => {
    if (!hasLoadedConnection) return;
    const inOnboarding = segments[0] === 'onboarding';

    if (!isConfigured && !inOnboarding) {
      router.replace('/onboarding');
    }
  }, [hasLoadedConnection, isConfigured, router, segments]);

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
