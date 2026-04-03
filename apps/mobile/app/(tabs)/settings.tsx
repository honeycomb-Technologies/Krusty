import { useState } from 'react';
import {
  View,
  Text,
  TextInput,
  Pressable,
  StyleSheet,
  ScrollView,
  Alert,
  ActivityIndicator,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import {
  Wifi,
  WifiOff,
  RefreshCw,
  LogOut,
  Moon,
  Sun,
  Monitor,
  Cpu,
  Link,
  X,
  Bell,
  BellOff,
  BellRing,
} from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import { useRouter } from 'expo-router';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { useNotifications, type NotificationLevel } from '../../hooks/useNotifications';
import { GlassCard } from '../../components/ui/GlassCard';
import type { ColorScheme } from '@krusty/ui';

export default function SettingsScreen() {
  const router = useRouter();
  const { theme, colorScheme, setColorScheme } = useThemeContext();
  const { isConnected, isConfigured, serverUrl, status, connect, disconnect, reconnect } = useConnection();
  const { notificationLevel, changeNotificationLevel, pushToken } = useNotifications();
  const [inputUrl, setInputUrl] = useState('');
  const [inputToken, setInputToken] = useState('');
  const [isConnecting, setIsConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const t = theme.colors;
  const g = theme.colors.glass;

  const handleConnect = async () => {
    if (!inputUrl.trim() || !inputToken.trim()) return;
    setIsConnecting(true);
    setConnectError(null);
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);

    const url = inputUrl.trim().replace(/\/+$/, '');
    const success = await connect(url, inputToken.trim());

    if (success) {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      setInputUrl('');
      setInputToken('');
    } else {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      setConnectError('Connection failed. Check URL and token.');
    }
    setIsConnecting(false);
  };

  const handleDisconnect = () => {
    Alert.alert('Disconnect', 'Remove saved server connection?', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Disconnect',
        style: 'destructive',
        onPress: () => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
          disconnect();
        },
      },
    ]);
  };

  const schemeOptions: { key: ColorScheme; label: string; icon: typeof Moon }[] = [
    { key: 'dark', label: 'Dark', icon: Moon },
    { key: 'light', label: 'Light', icon: Sun },
    { key: 'system', label: 'System', icon: Monitor },
  ];

  const notifOptions: { key: NotificationLevel; label: string; icon: typeof Bell }[] = [
    { key: 'all', label: 'All', icon: BellRing },
    { key: 'important', label: 'Important', icon: Bell },
    { key: 'silent', label: 'Silent', icon: BellOff },
  ];

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: t.background }]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <View style={styles.header}>
          <Text style={[styles.title, { color: t.foreground }]}>Settings</Text>
          <Pressable
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              router.back();
            }}
            style={[styles.closeBtn, { backgroundColor: t.border + '40' }]}
          >
            <X size={20} color={t.foreground} strokeWidth={2} />
          </Pressable>
        </View>

        {/* Connection */}
        <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>CONNECTION</Text>

        {isConfigured ? (
          <GlassCard>
            <View style={styles.row}>
              {isConnected ? (
                <Wifi size={20} color={t.success} strokeWidth={1.8} />
              ) : (
                <WifiOff size={20} color={t.error} strokeWidth={1.8} />
              )}
              <View style={styles.rowContent}>
                <Text style={[styles.rowTitle, { color: t.foreground }]}>
                  {isConnected ? 'Connected' : status === 'connecting' ? 'Connecting...' : 'Disconnected'}
                </Text>
                {serverUrl && (
                  <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]} numberOfLines={1}>
                    {serverUrl}
                  </Text>
                )}
              </View>
            </View>

            <View style={[styles.separator, { backgroundColor: t.border }]} />

            <View style={styles.actions}>
              <Pressable
                onPress={() => {
                  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  reconnect();
                }}
                style={styles.actionBtn}
              >
                <RefreshCw size={18} color={t.userMessage} strokeWidth={1.8} />
                <Text style={[styles.actionText, { color: t.userMessage }]}>Reconnect</Text>
              </Pressable>

              <Pressable onPress={handleDisconnect} style={styles.actionBtn}>
                <LogOut size={18} color={t.error} strokeWidth={1.8} />
                <Text style={[styles.actionText, { color: t.error }]}>Disconnect</Text>
              </Pressable>
            </View>
          </GlassCard>
        ) : (
          <GlassCard>
            <View style={styles.connectForm}>
              <View style={styles.row}>
                <Link size={20} color={t.mutedForeground} strokeWidth={1.8} />
                <Text style={[styles.rowTitle, { color: t.foreground }]}>Connect to Server</Text>
              </View>

              <View style={[styles.inputWrap, { backgroundColor: g.background, borderColor: g.border }]}>
                <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>SERVER URL</Text>
                <TextInput
                  style={[styles.input, { color: t.foreground }]}
                  value={inputUrl}
                  onChangeText={setInputUrl}
                  placeholder="https://device.tail123.ts.net:8443"
                  placeholderTextColor={t.mutedForeground + '60'}
                  autoCapitalize="none"
                  autoCorrect={false}
                  keyboardType="url"
                />
              </View>

              <View style={[styles.inputWrap, { backgroundColor: g.background, borderColor: g.border }]}>
                <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>TOKEN</Text>
                <TextInput
                  style={[styles.input, { color: t.foreground }]}
                  value={inputToken}
                  onChangeText={setInputToken}
                  placeholder="kr_remote_..."
                  placeholderTextColor={t.mutedForeground + '60'}
                  autoCapitalize="none"
                  autoCorrect={false}
                  secureTextEntry
                />
              </View>

              {connectError && (
                <Text style={[styles.errorText, { color: t.error }]}>{connectError}</Text>
              )}

              <Pressable
                onPress={handleConnect}
                disabled={isConnecting || !inputUrl.trim() || !inputToken.trim()}
                style={({ pressed }) => [
                  styles.connectBtn,
                  {
                    backgroundColor: pressed ? t.userMessage + 'cc' : t.userMessage,
                    opacity: isConnecting || !inputUrl.trim() || !inputToken.trim() ? 0.5 : 1,
                  },
                ]}
              >
                {isConnecting ? (
                  <ActivityIndicator color="#fff" size="small" />
                ) : (
                  <Text style={styles.connectBtnText}>Connect</Text>
                )}
              </Pressable>
            </View>
          </GlassCard>
        )}

        {/* Appearance */}
        <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>APPEARANCE</Text>
        <GlassCard>
          <View style={styles.schemeRow}>
            {schemeOptions.map(opt => {
              const Icon = opt.icon;
              const active = colorScheme === opt.key;
              return (
                <Pressable
                  key={opt.key}
                  onPress={() => {
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                    setColorScheme(opt.key);
                  }}
                  style={[
                    styles.schemeBtn,
                    {
                      backgroundColor: active ? t.userMessage + '20' : 'transparent',
                      borderColor: active ? t.userMessage : t.border,
                    },
                  ]}
                >
                  <Icon
                    size={18}
                    color={active ? t.userMessage : t.mutedForeground}
                    strokeWidth={1.8}
                  />
                  <Text
                    style={[
                      styles.schemeBtnText,
                      { color: active ? t.userMessage : t.mutedForeground },
                    ]}
                  >
                    {opt.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </GlassCard>

        {/* Notifications */}
        <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>NOTIFICATIONS</Text>
        <GlassCard>
          <View style={styles.row}>
            <Bell size={20} color={t.mutedForeground} strokeWidth={1.8} />
            <View style={styles.rowContent}>
              <Text style={[styles.rowTitle, { color: t.foreground }]}>Notification Level</Text>
              <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
                {notificationLevel === 'all' ? 'All events including Mako updates'
                  : notificationLevel === 'important' ? 'Tool approvals and completions only'
                  : 'No notifications'}
              </Text>
            </View>
          </View>
          <View style={[styles.separator, { backgroundColor: t.border }]} />
          <View style={styles.schemeRow}>
            {notifOptions.map(opt => {
              const Icon = opt.icon;
              const active = notificationLevel === opt.key;
              return (
                <Pressable
                  key={opt.key}
                  onPress={() => {
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                    changeNotificationLevel(opt.key);
                  }}
                  style={[
                    styles.schemeBtn,
                    {
                      backgroundColor: active ? t.userMessage + '20' : 'transparent',
                      borderColor: active ? t.userMessage : t.border,
                    },
                  ]}
                >
                  <Icon
                    size={18}
                    color={active ? t.userMessage : t.mutedForeground}
                    strokeWidth={1.8}
                  />
                  <Text
                    style={[
                      styles.schemeBtnText,
                      { color: active ? t.userMessage : t.mutedForeground },
                    ]}
                  >
                    {opt.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
          {pushToken && (
            <>
              <View style={[styles.separator, { backgroundColor: t.border }]} />
              <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]} numberOfLines={1}>
                Push: registered
              </Text>
            </>
          )}
        </GlassCard>

        {/* About */}
        <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>ABOUT</Text>
        <GlassCard>
          <View style={styles.row}>
            <Cpu size={20} color={t.mutedForeground} strokeWidth={1.8} />
            <View style={styles.rowContent}>
              <Text style={[styles.rowTitle, { color: t.foreground }]}>Krusty Mobile</Text>
              <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>v0.8.0</Text>
            </View>
          </View>
        </GlassCard>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 100,
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingVertical: 16,
    paddingHorizontal: 4,
  },
  title: {
    fontSize: 28,
    fontWeight: '700',
    letterSpacing: -0.5,
  },
  closeBtn: {
    width: 32,
    height: 32,
    borderRadius: 16,
    alignItems: 'center',
    justifyContent: 'center',
  },
  sectionLabel: {
    fontSize: 13,
    fontWeight: '600',
    letterSpacing: 0.5,
    marginTop: 24,
    marginBottom: 8,
    paddingHorizontal: 4,
  },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  rowContent: {
    flex: 1,
    gap: 2,
  },
  rowTitle: {
    fontSize: 17,
    fontWeight: '500',
  },
  rowSubtitle: {
    fontSize: 13,
  },
  separator: {
    height: StyleSheet.hairlineWidth,
    marginVertical: 12,
  },
  actions: {
    flexDirection: 'row',
    gap: 16,
  },
  actionBtn: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  actionText: {
    fontSize: 15,
    fontWeight: '500',
  },
  connectForm: {
    gap: 14,
  },
  inputWrap: {
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 12,
  },
  inputLabel: {
    fontSize: 11,
    fontWeight: '600',
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  input: {
    fontSize: 16,
    padding: 0,
  },
  errorText: {
    fontSize: 14,
    textAlign: 'center',
  },
  connectBtn: {
    borderRadius: 14,
    paddingVertical: 14,
    alignItems: 'center',
  },
  connectBtnText: {
    color: '#fff',
    fontSize: 17,
    fontWeight: '600',
  },
  schemeRow: {
    flexDirection: 'row',
    gap: 10,
  },
  schemeBtn: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
    paddingVertical: 10,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
  },
  schemeBtnText: {
    fontSize: 15,
    fontWeight: '500',
  },
});
