import { useEffect, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { router } from 'expo-router';
import { useConnection } from '@mobile/hooks/useConnection';
import { useThemeContext } from '@mobile/hooks/useTheme';
import * as Haptics from '@mobile/platform/haptics';

function inferDesktopServerUrl(): string {
  if (typeof window === 'undefined') return 'http://127.0.0.1:3000';
  const injected = (window as any).__KRUSTY_SERVER_URL;
  if (typeof injected === 'string' && injected.trim()) return injected.replace(/\/+$/, '');
  return 'http://127.0.0.1:3000';
}

export default function DesktopOnboardingScreen() {
  const { theme } = useThemeContext();
  const { connect, isConfigured } = useConnection();
  const [serverUrl, setServerUrl] = useState(inferDesktopServerUrl);
  const [token, setToken] = useState(
    typeof window !== 'undefined' && (window as any).__KRUSTY_SERVER_TOKEN
      ? String((window as any).__KRUSTY_SERVER_TOKEN)
      : 'local',
  );
  const [isConnecting, setIsConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autoTried, setAutoTried] = useState(false);
  const t = theme.colors;

  useEffect(() => {
    if (isConfigured) router.replace('/');
  }, [isConfigured]);

  const handleConnect = async () => {
    const url = serverUrl.trim().replace(/\/+$/, '');
    const nextToken = token.trim() || 'local';
    if (!url) return;
    setIsConnecting(true);
    setError(null);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const success = await connect(url, nextToken);
    if (success) {
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      router.replace('/');
    } else {
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      setError('Could not connect. Check your URL and token.');
    }
    setIsConnecting(false);
  };

  // Local desktop convenience: if URL/token are already filled, connect immediately.
  useEffect(() => {
    if (autoTried || isConfigured || isConnecting) return;
    const url = serverUrl.trim();
    const nextToken = token.trim() || 'local';
    if (!url) return;
    setAutoTried(true);
    void (async () => {
      setIsConnecting(true);
      const success = await connect(url, nextToken);
      if (success) {
        router.replace('/');
      } else {
        setError('Could not connect. Start the local Mitsuro server or check URL/token.');
      }
      setIsConnecting(false);
    })();
  }, [autoTried, connect, isConfigured, isConnecting, serverUrl, token]);

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: t.background }]}>
      <KeyboardAvoidingView
        style={styles.content}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
      >
        <View style={styles.header}>
          <Text style={[styles.title, { color: t.foreground }]}>Mitsuro Desktop</Text>
          <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
            Connect to your local or remote Mitsuro server
          </Text>
        </View>

        <View style={styles.form}>
          <View style={[styles.inputGroup, { backgroundColor: t.glass.background, borderColor: t.glass.border }]}>
            <Text style={[styles.label, { color: t.mutedForeground }]}>Server URL</Text>
            <TextInput
              style={[styles.input, { color: t.foreground }]}
              value={serverUrl}
              onChangeText={setServerUrl}
              placeholder="http://127.0.0.1:3000"
              placeholderTextColor={`${t.mutedForeground}60`}
              autoCapitalize="none"
              autoCorrect={false}
            />
          </View>
          <View style={[styles.inputGroup, { backgroundColor: t.glass.background, borderColor: t.glass.border }]}>
            <Text style={[styles.label, { color: t.mutedForeground }]}>Token</Text>
            <TextInput
              style={[styles.input, { color: t.foreground }]}
              value={token}
              onChangeText={setToken}
              placeholder="local"
              placeholderTextColor={`${t.mutedForeground}60`}
              autoCapitalize="none"
              autoCorrect={false}
              secureTextEntry
            />
          </View>
          {error ? <Text style={[styles.error, { color: t.error }]}>{error}</Text> : null}
          <Pressable
            onPress={() => void handleConnect()}
            disabled={isConnecting || !serverUrl.trim()}
            style={({ pressed }) => [
              styles.button,
              {
                backgroundColor: pressed ? `${t.userMessage}cc` : t.userMessage,
                opacity: isConnecting || !serverUrl.trim() ? 0.55 : 1,
              },
            ]}
          >
            {isConnecting ? (
              <ActivityIndicator color="#fff" />
            ) : (
              <Text style={styles.buttonText}>Connect</Text>
            )}
          </Pressable>
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  content: {
    flex: 1,
    justifyContent: 'center',
    paddingHorizontal: 28,
    maxWidth: 480,
    width: '100%',
    alignSelf: 'center',
  },
  header: { marginBottom: 28, gap: 6 },
  title: { fontSize: 28, fontWeight: '800', letterSpacing: -0.4 },
  subtitle: { fontSize: 14, lineHeight: 20 },
  form: { gap: 12 },
  inputGroup: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 10,
    gap: 6,
  },
  label: { fontSize: 11, fontWeight: '700', letterSpacing: 0.3, textTransform: 'uppercase' },
  input: { fontSize: 15, paddingVertical: 4 },
  error: { fontSize: 13 },
  button: {
    marginTop: 4,
    minHeight: 44,
    borderRadius: 12,
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonText: { color: '#fff', fontSize: 15, fontWeight: '700' },
});
