import { useState } from "react";
import {
  View,
  Text,
  TextInput,
  Pressable,
  StyleSheet,
  ScrollView,
  Modal,
  ActivityIndicator,
} from "react-native";
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
} from "lucide-react-native";
import * as Haptics from "../platform/haptics";
import { BlurView } from "../platform/blur";
import { useThemeContext } from "../hooks/useTheme";
import { useConnection } from "../hooks/useConnection";
import { GlassCard } from "./ui/GlassCard";
import type { ColorScheme } from "@krusty/ui";

interface SettingsModalProps {
  visible: boolean;
  onClose: () => void;
}

export function SettingsModal({ visible, onClose }: SettingsModalProps) {
  const { theme, colorScheme, setColorScheme } = useThemeContext();
  const {
    isConnected,
    isConfigured,
    serverUrl,
    status,
    connect,
    disconnect,
    reconnect,
  } = useConnection();
  const [inputUrl, setInputUrl] = useState("");
  const [inputToken, setInputToken] = useState("");
  const [isConnecting, setIsConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const t = theme.colors;
  const g = theme.colors.glass;
  const isDark = theme.scheme === "dark";

  const handleConnect = async () => {
    if (!inputUrl.trim() || !inputToken.trim()) return;
    setIsConnecting(true);
    setConnectError(null);
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const url = inputUrl.trim().replace(/\/+$/, "");
    const success = await connect(url, inputToken.trim());
    if (success) {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      setInputUrl("");
      setInputToken("");
    } else {
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      setConnectError("Connection failed. Check URL and token.");
    }
    setIsConnecting(false);
  };

  const handleDisconnect = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
    disconnect();
  };

  const schemeOptions: { key: ColorScheme; label: string; icon: typeof Moon }[] = [
    { key: "dark", label: "Dark", icon: Moon },
    { key: "light", label: "Light", icon: Sun },
    { key: "system", label: "System", icon: Monitor },
  ];

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View style={styles.panel}>
          <BlurView
            intensity={40}
            tint={isDark ? "systemChromeMaterialDark" : "systemChromeMaterialLight"}
            style={StyleSheet.absoluteFill}
          />
          <View
            style={[
              StyleSheet.absoluteFill,
              { backgroundColor: isDark ? "rgba(11,17,25,0.94)" : "rgba(255,255,255,0.94)" },
            ]}
          />

          {/* Header */}
          <View style={[styles.header, { borderBottomColor: t.border }]}>
            <Text style={[styles.headerTitle, { color: t.foreground }]}>Settings</Text>
            <Pressable onPress={onClose} style={[styles.closeBtn, { backgroundColor: `${t.border}40` }]}>
              <X size={18} color={t.foreground} strokeWidth={2} />
            </Pressable>
          </View>

          <ScrollView contentContainerStyle={styles.content} showsVerticalScrollIndicator={false}>
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
                      {isConnected ? "Connected" : status === "connecting" ? "Connecting..." : "Disconnected"}
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
                    onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); reconnect(); }}
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
                      placeholderTextColor={`${t.mutedForeground}60`}
                      autoCapitalize="none"
                      autoCorrect={false}
                    />
                  </View>
                  <View style={[styles.inputWrap, { backgroundColor: g.background, borderColor: g.border }]}>
                    <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>TOKEN</Text>
                    <TextInput
                      style={[styles.input, { color: t.foreground }]}
                      value={inputToken}
                      onChangeText={setInputToken}
                      placeholder="kr_remote_..."
                      placeholderTextColor={`${t.mutedForeground}60`}
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
                    style={[
                      styles.connectBtn,
                      { backgroundColor: t.userMessage, opacity: isConnecting || !inputUrl.trim() || !inputToken.trim() ? 0.5 : 1 },
                    ]}
                  >
                    {isConnecting ? <ActivityIndicator color="#fff" size="small" /> : <Text style={styles.connectBtnText}>Connect</Text>}
                  </Pressable>
                </View>
              </GlassCard>
            )}

            {/* Appearance */}
            <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>APPEARANCE</Text>
            <GlassCard>
              <View style={styles.schemeRow}>
                {schemeOptions.map((opt) => {
                  const Icon = opt.icon;
                  const active = colorScheme === opt.key;
                  return (
                    <Pressable
                      key={opt.key}
                      onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setColorScheme(opt.key); }}
                      style={[styles.schemeBtn, { backgroundColor: active ? `${t.userMessage}20` : "transparent", borderColor: active ? t.userMessage : t.border }]}
                    >
                      <Icon size={18} color={active ? t.userMessage : t.mutedForeground} strokeWidth={1.8} />
                      <Text style={[styles.schemeBtnText, { color: active ? t.userMessage : t.mutedForeground }]}>{opt.label}</Text>
                    </Pressable>
                  );
                })}
              </View>
            </GlassCard>

            {/* About */}
            <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>ABOUT</Text>
            <GlassCard>
              <View style={styles.row}>
                <Cpu size={20} color={t.mutedForeground} strokeWidth={1.8} />
                <View style={styles.rowContent}>
                  <Text style={[styles.rowTitle, { color: t.foreground }]}>Krusty</Text>
                  <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>v0.9.0</Text>
                </View>
              </View>
            </GlassCard>
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1, backgroundColor: "rgba(0,0,0,0.5)", justifyContent: "center", alignItems: "center", padding: 32 },
  panel: { width: "100%", maxWidth: 480, maxHeight: "80%", borderRadius: 20, overflow: "hidden" },
  header: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingHorizontal: 20, paddingVertical: 16, borderBottomWidth: StyleSheet.hairlineWidth },
  headerTitle: { fontSize: 20, fontWeight: "700" },
  closeBtn: { width: 32, height: 32, borderRadius: 16, alignItems: "center", justifyContent: "center" },
  content: { padding: 20, paddingBottom: 40 },
  sectionLabel: { fontSize: 13, fontWeight: "600", letterSpacing: 0.5, marginTop: 20, marginBottom: 8, paddingHorizontal: 4 },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  rowContent: { flex: 1, gap: 2 },
  rowTitle: { fontSize: 17, fontWeight: "500" },
  rowSubtitle: { fontSize: 13 },
  separator: { height: StyleSheet.hairlineWidth, marginVertical: 12 },
  actions: { flexDirection: "row", gap: 16 },
  actionBtn: { flexDirection: "row", alignItems: "center", gap: 6 },
  actionText: { fontSize: 15, fontWeight: "500" },
  connectForm: { gap: 14 },
  inputWrap: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, padding: 12 },
  inputLabel: { fontSize: 11, fontWeight: "600", letterSpacing: 0.5, marginBottom: 6 },
  input: { fontSize: 16, padding: 0 },
  errorText: { fontSize: 14, textAlign: "center" },
  connectBtn: { borderRadius: 14, paddingVertical: 14, alignItems: "center" },
  connectBtnText: { color: "#fff", fontSize: 17, fontWeight: "600" },
  schemeRow: { flexDirection: "row", gap: 10 },
  schemeBtn: { flex: 1, flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 6, paddingVertical: 10, borderRadius: 12, borderWidth: StyleSheet.hairlineWidth },
  schemeBtnText: { fontSize: 15, fontWeight: "500" },
});
