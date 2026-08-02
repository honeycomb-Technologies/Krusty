import { useEffect, useState } from 'react';
import { ActivityIndicator, LogBox, StyleSheet, Text, View } from 'react-native';
import { Stack, useRouter, useSegments } from 'expo-router';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';

import { ThemeProvider, useThemeContext } from '@mobile/hooks/useTheme';
import { ConnectionProvider, useConnection } from '@mobile/hooks/useConnection';
import { StoresProvider, useStores } from '@mobile/hooks/useStores';
import { ensureDesktopServerGlobals } from '../src/bootstrap/desktopConnection';

const BOOT_BACKGROUND = '#0b1119';

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
  const { client, isConfigured, status } = useConnection();
  const stores = useStores();
  const router = useRouter();
  const segments = useSegments();

  useEffect(() => {
    const inOnboarding = segments[0] === 'onboarding';
    // Only force onboarding when truly unconfigured after bootstrap.
    if (!isConfigured && !inOnboarding) {
      router.replace('/onboarding');
    } else if (isConfigured && inOnboarding) {
      router.replace('/');
    }
  }, [isConfigured, router, segments]);

  // Keep a calm boot surface until stores are ready for the main app.
  if (isConfigured && !stores && segments[0] !== 'onboarding') {
    return (
      <View style={[styles.boot, { backgroundColor: theme.colors.background }]}>
        <ActivityIndicator color={theme.colors.userMessage} />
        <Text style={[styles.bootText, { color: theme.colors.mutedForeground }]}>
          {status === 'connecting' ? 'Connecting…' : 'Preparing desktop workspace…'}
        </Text>
      </View>
    );
  }

  return (
    <View style={[styles.root, { backgroundColor: theme.colors.background }]}>
      <StatusBar style={theme.scheme === 'dark' ? 'light' : 'dark'} />
      <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: theme.colors.background },
        }}
      >
        <Stack.Screen name="index" />
        <Stack.Screen name="onboarding" />
      </Stack>
    </View>
  );
}

function DesktopProviders({ children }: { children: React.ReactNode }) {
  const { client } = useConnection();
  return <StoresProvider client={client}>{children}</StoresProvider>;
}

export default function RootLayout() {
  const [bootstrapped, setBootstrapped] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await ensureDesktopServerGlobals();
      } finally {
        if (!cancelled) setBootstrapped(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!bootstrapped) {
    return (
      <View style={styles.boot}>
        <ActivityIndicator color="#75617e" />
        <Text style={styles.bootText}>Starting Mitsuro Desktop…</Text>
      </View>
    );
  }

  return (
    <GestureHandlerRootView style={styles.root}>
      <SafeAreaProvider>
        <ThemeProvider>
          <ConnectionProvider>
            <DesktopProviders>
              <RootNavigator />
            </DesktopProviders>
          </ConnectionProvider>
        </ThemeProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: BOOT_BACKGROUND,
  },
  boot: {
    flex: 1,
    backgroundColor: BOOT_BACKGROUND,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 12,
  },
  bootText: {
    color: '#a1a1aa',
    fontSize: 13,
    fontWeight: '600',
  },
});
