import { useEffect } from 'react';
import { StatusBar } from 'expo-status-bar';
import { Stack, useRouter, useSegments } from 'expo-router';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { StyleSheet } from 'react-native';
import { ThemeProvider, useThemeContext } from '../hooks/useTheme';
import { ConnectionProvider, useConnection } from '../hooks/useConnection';
import { StoresProvider } from '../hooks/useStores';
import { useDeepLink } from '../hooks/useDeepLink';
import { SplashProvider, useSplashState } from '../hooks/useSplashState';
import { SplashOverlay } from '../components/splash/SplashOverlay';

function RootNavigator() {
  const { theme } = useThemeContext();
  const { client, isConfigured } = useConnection();
  useDeepLink();
  const router = useRouter();
  const segments = useSegments();

  useEffect(() => {
    const inOnboarding = segments[0] === 'onboarding';

    if (!isConfigured && !inOnboarding) {
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

  return <StoresProvider client={client}>{content}</StoresProvider>;
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
                <RootNavigator />
              </ConnectionProvider>
            </ThemeProvider>
          </SplashWrapper>
        </SplashProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
});
