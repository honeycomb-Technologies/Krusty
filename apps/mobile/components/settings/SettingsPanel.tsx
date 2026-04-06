import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  Bell,
  BellOff,
  BellRing,
  Cpu,
  ExternalLink,
  Link,
  LogOut,
  Monitor,
  Moon,
  RefreshCw,
  Sun,
  Wifi,
  WifiOff,
  X,
} from "lucide-react-native";

import type {
  McpServerResponse,
  OAuthStartResponse,
  PortEntry,
  PreviewSettings,
  PreviewSettingsPatch,
  ProviderStatus,
  SkillInfo,
} from "@krusty/api";
import type { ColorScheme } from "@krusty/ui";

import * as Haptics from "../../platform/haptics";
import { openURL } from "../../platform/linking";
import { useConnection } from "../../hooks/useConnection";
import { useNotifications, type NotificationLevel } from "../../hooks/useNotifications";
import { useThemeContext } from "../../hooks/useTheme";
import { GlassCard } from "../ui/GlassCard";

type ProviderFormState = Record<string, string>;
type PreviewDraftState = {
  autoRefreshSecs: string;
  probeTimeoutMs: string;
};

interface ActiveOAuthFlow {
  provider: string;
  flowType: OAuthStartResponse["flow_type"];
  authUrl: string;
  pasteCode: boolean;
  userCode?: string | null;
  verificationUriComplete?: string | null;
}

interface SettingsPanelProps {
  active?: boolean;
  onClose?: () => void;
  showHeader?: boolean;
}

const OAUTH_POLL_INTERVAL_MS = 2_000;
const OAUTH_POLL_TIMEOUT_MS = 120_000;

function toErrorMessage(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

function SectionTitle({ title, subtitle }: { title: string; subtitle?: string }) {
  const { theme } = useThemeContext();
  return (
    <View style={styles.sectionHeader}>
      <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}>{title}</Text>
      {subtitle ? (
        <Text style={[styles.sectionSubtitle, { color: theme.colors.mutedForeground }]}>
          {subtitle}
        </Text>
      ) : null}
    </View>
  );
}

function Pill({
  label,
  tone = "neutral",
}: {
  label: string;
  tone?: "neutral" | "success" | "warning" | "error" | "info";
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const palette = {
    neutral: { backgroundColor: `${t.border}35`, color: t.mutedForeground },
    success: { backgroundColor: `${t.success}20`, color: t.success },
    warning: { backgroundColor: `${t.warning}20`, color: t.warning },
    error: { backgroundColor: `${t.error}20`, color: t.error },
    info: { backgroundColor: `${t.userMessage}18`, color: t.userMessage },
  } as const;
  const selected = palette[tone];

  return (
    <View style={[styles.pill, { backgroundColor: selected.backgroundColor }]}>
      <Text style={[styles.pillText, { color: selected.color }]}>{label}</Text>
    </View>
  );
}

function MessageBanner({
  text,
  tone = "error",
}: {
  text: string | null;
  tone?: "error" | "info";
}) {
  const { theme } = useThemeContext();
  if (!text) return null;

  const color =
    tone === "error" ? theme.colors.error : theme.colors.mutedForeground;
  const bg =
    tone === "error"
      ? `${theme.colors.error}12`
      : `${theme.colors.userMessage}12`;

  return (
    <View style={[styles.banner, { backgroundColor: bg, borderColor: `${color}20` }]}>
      <Text style={[styles.bannerText, { color }]}>{text}</Text>
    </View>
  );
}

function previewStatusText(port: PortEntry): string {
  if (!port.active) return "Offline";
  if (port.is_previewable_http) {
    return port.last_probe_ms ? `HTTP ready • ${port.last_probe_ms}ms` : "HTTP ready";
  }

  switch (port.probe_status) {
    case "timeout":
      return "Probe timeout";
    case "conn_refused":
      return "Connection refused";
    case "non_http":
      return "Non-HTTP listener";
    default:
      return "Probe failed";
  }
}

async function pollOAuthUntilDone(
  provider: string,
  getOAuthStatus: (provider: string) => Promise<{ has_token: boolean; flow_active: boolean }>,
) {
  const deadline = Date.now() + OAUTH_POLL_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const status = await getOAuthStatus(provider);
    if (status.has_token || !status.flow_active) {
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, OAUTH_POLL_INTERVAL_MS));
  }

  return getOAuthStatus(provider);
}

export function SettingsPanel({
  active = true,
  onClose,
  showHeader = true,
}: SettingsPanelProps) {
  const { theme, colorScheme, setColorScheme } = useThemeContext();
  const {
    client,
    isConnected,
    isConfigured,
    serverUrl,
    status,
    connect,
    disconnect,
    reconnect,
  } = useConnection();
  const { notificationLevel, changeNotificationLevel, pushToken } =
    useNotifications();

  const [inputUrl, setInputUrl] = useState("");
  const [inputToken, setInputToken] = useState("");
  const [isConnecting, setIsConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [providerForms, setProviderForms] = useState<ProviderFormState>({});
  const [providersLoading, setProvidersLoading] = useState(false);
  const [providerBusyKey, setProviderBusyKey] = useState<string | null>(null);
  const [providerMessage, setProviderMessage] = useState<string | null>(null);
  const [activeOAuthFlow, setActiveOAuthFlow] = useState<ActiveOAuthFlow | null>(null);
  const [oauthCode, setOauthCode] = useState("");

  const [mcpServers, setMcpServers] = useState<McpServerResponse[]>([]);
  const [mcpLoading, setMcpLoading] = useState(false);
  const [mcpBusyKey, setMcpBusyKey] = useState<string | null>(null);
  const [mcpMessage, setMcpMessage] = useState<string | null>(null);

  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsMessage, setSkillsMessage] = useState<string | null>(null);

  const [previewSettings, setPreviewSettings] = useState<PreviewSettings | null>(null);
  const [previewPorts, setPreviewPorts] = useState<PortEntry[]>([]);
  const [previewDraft, setPreviewDraft] = useState<PreviewDraftState>({
    autoRefreshSecs: "",
    probeTimeoutMs: "",
  });
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewBusyKey, setPreviewBusyKey] = useState<string | null>(null);
  const [previewMessage, setPreviewMessage] = useState<string | null>(null);

  const t = theme.colors;
  const g = theme.colors.glass;

  const schemeOptions: { key: ColorScheme; label: string; icon: typeof Moon }[] =
    useMemo(
      () => [
        { key: "dark", label: "Dark", icon: Moon },
        { key: "light", label: "Light", icon: Sun },
        { key: "system", label: "System", icon: Monitor },
      ],
      [],
    );

  const notifOptions: {
    key: NotificationLevel;
    label: string;
    icon: typeof Bell;
  }[] = useMemo(
    () => [
      { key: "all", label: "All", icon: BellRing },
      { key: "important", label: "Important", icon: Bell },
      { key: "silent", label: "Silent", icon: BellOff },
    ],
    [],
  );

  const loadProviders = useCallback(async () => {
    if (!client) {
      setProviders([]);
      return;
    }

    setProvidersLoading(true);
    try {
      const nextProviders = await client.getCredentials();
      setProviders(nextProviders);
      setProviderMessage(null);
    } catch (err) {
      setProviderMessage(toErrorMessage(err, "Failed to load provider settings."));
    } finally {
      setProvidersLoading(false);
    }
  }, [client]);

  const loadMcpServers = useCallback(async () => {
    if (!client) {
      setMcpServers([]);
      return;
    }

    setMcpLoading(true);
    try {
      const nextServers = await client.getMcpServers();
      setMcpServers(nextServers);
      setMcpMessage(null);
    } catch (err) {
      setMcpMessage(toErrorMessage(err, "Failed to load MCP servers."));
    } finally {
      setMcpLoading(false);
    }
  }, [client]);

  const loadSkills = useCallback(async () => {
    if (!client) {
      setSkills([]);
      return;
    }

    setSkillsLoading(true);
    try {
      const nextSkills = await client.getSkills();
      setSkills(nextSkills);
      setSkillsMessage(null);
    } catch (err) {
      setSkillsMessage(toErrorMessage(err, "Failed to load skills."));
    } finally {
      setSkillsLoading(false);
    }
  }, [client]);

  const loadPreview = useCallback(async () => {
    if (!client) {
      setPreviewSettings(null);
      setPreviewPorts([]);
      return;
    }

    setPreviewLoading(true);
    try {
      const response = await client.getPorts();
      setPreviewSettings(response.settings);
      setPreviewPorts(response.ports);
      setPreviewDraft({
        autoRefreshSecs: String(response.settings.auto_refresh_secs),
        probeTimeoutMs: String(response.settings.probe_timeout_ms),
      });
      setPreviewMessage(response.discovery_error ?? null);
    } catch (err) {
      setPreviewMessage(toErrorMessage(err, "Failed to load preview settings."));
    } finally {
      setPreviewLoading(false);
    }
  }, [client]);

  const refreshOperationalState = useCallback(async () => {
    if (!client || !isConnected) return;
    await Promise.all([
      loadProviders(),
      loadMcpServers(),
      loadSkills(),
      loadPreview(),
    ]);
  }, [client, isConnected, loadMcpServers, loadPreview, loadProviders, loadSkills]);

  useEffect(() => {
    if (!active || !client || !isConnected) return;
    void refreshOperationalState();
  }, [active, client, isConnected, refreshOperationalState]);

  const handleConnect = useCallback(async () => {
    if (!inputUrl.trim() || !inputToken.trim()) return;

    setIsConnecting(true);
    setConnectError(null);
    await Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);

    const url = inputUrl.trim().replace(/\/+$/, "");
    const success = await connect(url, inputToken.trim());

    if (success) {
      await Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      setInputUrl("");
      setInputToken("");
    } else {
      await Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      setConnectError("Connection failed. Check URL and token.");
    }

    setIsConnecting(false);
  }, [connect, inputToken, inputUrl]);

  const handleDisconnect = useCallback(() => {
    Alert.alert("Disconnect", "Remove saved server connection?", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Disconnect",
        style: "destructive",
        onPress: () => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
          disconnect();
        },
      },
    ]);
  }, [disconnect]);

  const updateProviderForm = useCallback((providerId: string, value: string) => {
    setProviderForms((current) => ({ ...current, [providerId]: value }));
  }, []);

  const handleSaveCredential = useCallback(
    async (providerId: string) => {
      if (!client) return;
      const apiKey = providerForms[providerId]?.trim();
      if (!apiKey) return;

      setProviderBusyKey(`save:${providerId}`);
      try {
        await client.setCredential(providerId, apiKey);
        setProviderMessage(`${providerId} API key saved.`);
        setProviderForms((current) => ({ ...current, [providerId]: "" }));
        await loadProviders();
      } catch (err) {
        setProviderMessage(toErrorMessage(err, `Failed to save ${providerId} credential.`));
      } finally {
        setProviderBusyKey(null);
      }
    },
    [client, loadProviders, providerForms],
  );

  const handleDeleteCredential = useCallback(
    async (providerId: string) => {
      if (!client) return;

      setProviderBusyKey(`delete:${providerId}`);
      try {
        await client.deleteCredential(providerId);
        setProviderMessage(`${providerId} API key removed.`);
        await loadProviders();
      } catch (err) {
        setProviderMessage(toErrorMessage(err, `Failed to delete ${providerId} credential.`));
      } finally {
        setProviderBusyKey(null);
      }
    },
    [client, loadProviders],
  );

  const handleStartOAuth = useCallback(
    async (providerId: string) => {
      if (!client) return;

      setProviderBusyKey(`oauth:${providerId}`);
      setProviderMessage(null);
      setOauthCode("");

      try {
        const flow = await client.startOAuth(providerId);
        setActiveOAuthFlow({
          provider: flow.provider,
          flowType: flow.flow_type,
          authUrl: flow.auth_url,
          pasteCode: flow.paste_code,
          userCode: flow.device_code?.user_code ?? null,
          verificationUriComplete: flow.device_code?.verification_uri_complete ?? null,
        });

        await openURL(flow.device_code?.verification_uri_complete ?? flow.auth_url);

        if (!flow.paste_code) {
          const status = await pollOAuthUntilDone(
            flow.provider,
            client.getOAuthStatus.bind(client),
          );
          if (status.has_token) {
            setProviderMessage(`${flow.provider} OAuth connected.`);
            setActiveOAuthFlow(null);
            await loadProviders();
          } else {
            setProviderMessage(`${flow.provider} OAuth is still pending.`);
          }
        }
      } catch (err) {
        setProviderMessage(toErrorMessage(err, `Failed to start ${providerId} OAuth.`));
      } finally {
        setProviderBusyKey(null);
      }
    },
    [client, loadProviders],
  );

  const handleExchangeOAuthCode = useCallback(async () => {
    if (!client || !activeOAuthFlow || !oauthCode.trim()) return;

    setProviderBusyKey(`exchange:${activeOAuthFlow.provider}`);
    try {
      await client.exchangeOAuthCode(activeOAuthFlow.provider, oauthCode.trim());
      setProviderMessage(`${activeOAuthFlow.provider} OAuth connected.`);
      setActiveOAuthFlow(null);
      setOauthCode("");
      await loadProviders();
    } catch (err) {
      setProviderMessage(toErrorMessage(err, "Failed to exchange OAuth code."));
    } finally {
      setProviderBusyKey(null);
    }
  }, [activeOAuthFlow, client, loadProviders, oauthCode]);

  const handleRevokeOAuth = useCallback(
    async (providerId: string) => {
      if (!client) return;

      setProviderBusyKey(`revoke:${providerId}`);
      try {
        await client.revokeOAuth(providerId);
        setProviderMessage(`${providerId} OAuth revoked.`);
        if (activeOAuthFlow?.provider === providerId) {
          setActiveOAuthFlow(null);
          setOauthCode("");
        }
        await loadProviders();
      } catch (err) {
        setProviderMessage(toErrorMessage(err, `Failed to revoke ${providerId} OAuth.`));
      } finally {
        setProviderBusyKey(null);
      }
    },
    [activeOAuthFlow?.provider, client, loadProviders],
  );

  const handleReloadMcp = useCallback(async () => {
    if (!client) return;

    setMcpBusyKey("reload");
    try {
      const nextServers = await client.reloadMcpConfig();
      setMcpServers(nextServers);
      setMcpMessage(null);
    } catch (err) {
      setMcpMessage(toErrorMessage(err, "Failed to reload MCP configuration."));
    } finally {
      setMcpBusyKey(null);
    }
  }, [client]);

  const handleToggleMcp = useCallback(
    async (server: McpServerResponse) => {
      if (!client) return;

      setMcpBusyKey(server.name);
      try {
        const updated = server.connected
          ? await client.disconnectMcpServer(server.name)
          : await client.connectMcpServer(server.name);
        setMcpServers((current) =>
          current.map((entry) => (entry.name === updated.name ? updated : entry)),
        );
      } catch (err) {
        setMcpMessage(toErrorMessage(err, `Failed to update MCP server ${server.name}.`));
      } finally {
        setMcpBusyKey(null);
      }
    },
    [client],
  );

  const handleUpdatePreviewToggle = useCallback(
    async (patch: PreviewSettingsPatch) => {
      if (!client || !previewSettings) return;

      setPreviewBusyKey("toggle");
      try {
        const nextSettings = await client.updatePreviewSettings(patch);
        setPreviewSettings(nextSettings);
        await loadPreview();
      } catch (err) {
        setPreviewMessage(toErrorMessage(err, "Failed to update preview settings."));
      } finally {
        setPreviewBusyKey(null);
      }
    },
    [client, loadPreview, previewSettings],
  );

  const handleSavePreviewNumbers = useCallback(async () => {
    if (!client) return;

    const autoRefreshSecs = Number.parseInt(previewDraft.autoRefreshSecs, 10);
    const probeTimeoutMs = Number.parseInt(previewDraft.probeTimeoutMs, 10);
    if (!Number.isFinite(autoRefreshSecs) || !Number.isFinite(probeTimeoutMs)) {
      setPreviewMessage("Auto refresh and probe timeout must be valid numbers.");
      return;
    }

    setPreviewBusyKey("numbers");
    try {
      const nextSettings = await client.updatePreviewSettings({
        auto_refresh_secs: autoRefreshSecs,
        probe_timeout_ms: probeTimeoutMs,
      });
      setPreviewSettings(nextSettings);
      await loadPreview();
      setPreviewMessage(null);
    } catch (err) {
      setPreviewMessage(toErrorMessage(err, "Failed to save preview timing settings."));
    } finally {
      setPreviewBusyKey(null);
    }
  }, [client, loadPreview, previewDraft.autoRefreshSecs, previewDraft.probeTimeoutMs]);

  const handleTogglePinnedPort = useCallback(
    async (port: PortEntry) => {
      if (!client) return;

      setPreviewBusyKey(`pin:${port.port}`);
      try {
        if (port.pinned) {
          await client.removePinnedPort(port.port);
        } else {
          await client.addPinnedPort(port.port);
        }
        await loadPreview();
      } catch (err) {
        setPreviewMessage(toErrorMessage(err, `Failed to update port ${port.port}.`));
      } finally {
        setPreviewBusyKey(null);
      }
    },
    [client, loadPreview],
  );

  const handleHidePort = useCallback(
    async (port: PortEntry) => {
      if (!client) return;

      setPreviewBusyKey(`hide:${port.port}`);
      try {
        await client.addHiddenPort(port.port);
        await loadPreview();
      } catch (err) {
        setPreviewMessage(toErrorMessage(err, `Failed to hide port ${port.port}.`));
      } finally {
        setPreviewBusyKey(null);
      }
    },
    [client, loadPreview],
  );

  return (
    <ScrollView
      contentContainerStyle={styles.content}
      keyboardShouldPersistTaps="handled"
      showsVerticalScrollIndicator={false}
    >
      {showHeader ? (
        <View style={styles.header}>
          <Text style={[styles.title, { color: t.foreground }]}>Settings</Text>
          {onClose ? (
            <Pressable
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onClose();
              }}
              style={[styles.closeBtn, { backgroundColor: `${t.border}40` }]}
            >
              <X size={18} color={t.foreground} strokeWidth={2} />
            </Pressable>
          ) : null}
        </View>
      ) : null}

      <SectionTitle
        title="Connection"
        subtitle="Server URL and remote session bootstrap"
      />
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
                {isConnected
                  ? "Connected"
                  : status === "connecting"
                    ? "Connecting..."
                    : "Disconnected"}
              </Text>
              {serverUrl ? (
                <Text
                  style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                  numberOfLines={1}
                >
                  {serverUrl}
                </Text>
              ) : null}
            </View>
          </View>

          <View style={[styles.separator, { backgroundColor: t.border }]} />

          <View style={styles.actions}>
            <Pressable
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                void reconnect();
              }}
              style={styles.actionBtn}
            >
              <RefreshCw size={18} color={t.userMessage} strokeWidth={1.8} />
              <Text style={[styles.actionText, { color: t.userMessage }]}>
                Reconnect
              </Text>
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
              <Text style={[styles.rowTitle, { color: t.foreground }]}>
                Connect to Server
              </Text>
            </View>

            <View
              style={[
                styles.inputWrap,
                { backgroundColor: g.background, borderColor: g.border },
              ]}
            >
              <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>
                SERVER URL
              </Text>
              <TextInput
                style={[styles.input, { color: t.foreground }]}
                value={inputUrl}
                onChangeText={setInputUrl}
                placeholder="https://device.tail123.ts.net:8443"
                placeholderTextColor={`${t.mutedForeground}60`}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
              />
            </View>

            <View
              style={[
                styles.inputWrap,
                { backgroundColor: g.background, borderColor: g.border },
              ]}
            >
              <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>
                TOKEN
              </Text>
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

            <MessageBanner text={connectError} />

            <Pressable
              onPress={handleConnect}
              disabled={isConnecting || !inputUrl.trim() || !inputToken.trim()}
              style={[
                styles.connectBtn,
                {
                  backgroundColor: t.userMessage,
                  opacity:
                    isConnecting || !inputUrl.trim() || !inputToken.trim() ? 0.5 : 1,
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

      <SectionTitle
        title="Providers & Auth"
        subtitle="API keys, OAuth, and provider status"
      />
      <GlassCard>
        {!isConnected ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            Connect to a server to manage provider credentials.
          </Text>
        ) : providersLoading ? (
          <View style={styles.loadingRow}>
            <ActivityIndicator color={t.userMessage} size="small" />
            <Text style={[styles.loadingText, { color: t.mutedForeground }]}>
              Loading providers…
            </Text>
          </View>
        ) : (
          <View style={styles.stack}>
            <MessageBanner
              text={providerMessage}
              tone={
                providerMessage?.includes("saved") ||
                providerMessage?.includes("connected") ||
                providerMessage?.includes("revoked")
                  ? "info"
                  : "error"
              }
            />
            {providers.map((provider) => {
              const draft = providerForms[provider.id] ?? "";
              return (
                <View
                  key={provider.id}
                  style={[styles.subsection, { borderColor: t.border }]}
                >
                  <View style={styles.subsectionHeader}>
                    <View style={styles.rowContent}>
                      <Text style={[styles.rowTitle, { color: t.foreground }]}>
                        {provider.name}
                      </Text>
                      <Text
                        style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                      >
                        id: {provider.id}
                      </Text>
                    </View>
                    <View style={styles.pillRow}>
                      <Pill
                        label={provider.configured ? "API key" : "No key"}
                        tone={provider.configured ? "success" : "neutral"}
                      />
                      {provider.supports_oauth ? (
                        <Pill
                          label={provider.has_oauth ? "OAuth" : "OAuth available"}
                          tone={provider.has_oauth ? "info" : "warning"}
                        />
                      ) : null}
                    </View>
                  </View>

                  <View
                    style={[
                      styles.inputWrap,
                      { backgroundColor: g.background, borderColor: g.border },
                    ]}
                  >
                    <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>
                      API KEY
                    </Text>
                    <TextInput
                      style={[styles.input, { color: t.foreground }]}
                      value={draft}
                      onChangeText={(value) => updateProviderForm(provider.id, value)}
                      placeholder={`Set ${provider.name} key`}
                      placeholderTextColor={`${t.mutedForeground}60`}
                      autoCapitalize="none"
                      autoCorrect={false}
                      secureTextEntry
                    />
                  </View>

                  <View style={styles.actionsWrap}>
                    <Pressable
                      onPress={() => void handleSaveCredential(provider.id)}
                      disabled={!draft.trim() || providerBusyKey === `save:${provider.id}`}
                      style={[styles.smallActionBtn, { borderColor: t.border }]}
                    >
                      {providerBusyKey === `save:${provider.id}` ? (
                        <ActivityIndicator color={t.userMessage} size="small" />
                      ) : (
                        <Text style={[styles.smallActionText, { color: t.userMessage }]}>
                          Save Key
                        </Text>
                      )}
                    </Pressable>

                    {provider.configured ? (
                      <Pressable
                        onPress={() => void handleDeleteCredential(provider.id)}
                        disabled={providerBusyKey === `delete:${provider.id}`}
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.error }]}>
                          Delete Key
                        </Text>
                      </Pressable>
                    ) : null}

                    {provider.supports_oauth ? (
                      <Pressable
                        onPress={() => void handleStartOAuth(provider.id)}
                        disabled={providerBusyKey === `oauth:${provider.id}`}
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.foreground }]}>
                          Start OAuth
                        </Text>
                      </Pressable>
                    ) : null}

                    {provider.has_oauth ? (
                      <Pressable
                        onPress={() => void handleRevokeOAuth(provider.id)}
                        disabled={providerBusyKey === `revoke:${provider.id}`}
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.warning }]}>
                          Revoke OAuth
                        </Text>
                      </Pressable>
                    ) : null}
                  </View>
                </View>
              );
            })}

            {activeOAuthFlow ? (
              <View style={[styles.subsection, { borderColor: t.border }]}>
                <Text style={[styles.rowTitle, { color: t.foreground }]}>
                  OAuth in progress: {activeOAuthFlow.provider}
                </Text>
                <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
                  Flow: {activeOAuthFlow.flowType.replace(/_/g, " ")}
                </Text>
                {activeOAuthFlow.userCode ? (
                  <Text style={[styles.oauthCodeHint, { color: t.foreground }]}>
                    Device code: {activeOAuthFlow.userCode}
                  </Text>
                ) : null}
                {activeOAuthFlow.pasteCode ? (
                  <>
                    <TextInput
                      style={[
                        styles.input,
                        styles.inlineInput,
                        {
                          color: t.foreground,
                          borderColor: g.border,
                          backgroundColor: g.background,
                        },
                      ]}
                      value={oauthCode}
                      onChangeText={setOauthCode}
                      placeholder="Paste OAuth code"
                      placeholderTextColor={`${t.mutedForeground}60`}
                      autoCapitalize="none"
                      autoCorrect={false}
                    />
                    <View style={styles.actionsWrap}>
                      <Pressable
                        onPress={() => void handleExchangeOAuthCode()}
                        disabled={
                          !oauthCode.trim() ||
                          providerBusyKey === `exchange:${activeOAuthFlow.provider}`
                        }
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.userMessage }]}>
                          Exchange Code
                        </Text>
                      </Pressable>
                      <Pressable
                        onPress={() =>
                          void openURL(
                            activeOAuthFlow.verificationUriComplete ??
                              activeOAuthFlow.authUrl,
                          )
                        }
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <ExternalLink
                          size={14}
                          color={t.foreground}
                          strokeWidth={1.8}
                        />
                        <Text style={[styles.smallActionText, { color: t.foreground }]}>
                          Open Auth
                        </Text>
                      </Pressable>
                    </View>
                  </>
                ) : (
                  <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
                    Finish the sign-in flow in the browser. This panel will refresh
                    provider status automatically.
                  </Text>
                )}
              </View>
            ) : null}
          </View>
        )}
      </GlassCard>

      <SectionTitle
        title="MCP"
        subtitle="Connected model-context servers and tool exposure"
      />
      <GlassCard>
        {!isConnected ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            Connect to a server to inspect MCP servers.
          </Text>
        ) : mcpLoading ? (
          <View style={styles.loadingRow}>
            <ActivityIndicator color={t.userMessage} size="small" />
            <Text style={[styles.loadingText, { color: t.mutedForeground }]}>
              Loading MCP servers…
            </Text>
          </View>
        ) : (
          <View style={styles.stack}>
            <MessageBanner text={mcpMessage} />
            <View style={styles.actionsWrap}>
              <Pressable
                onPress={() => void handleReloadMcp()}
                disabled={mcpBusyKey === "reload"}
                style={[styles.smallActionBtn, { borderColor: t.border }]}
              >
                <RefreshCw size={14} color={t.foreground} strokeWidth={1.8} />
                <Text style={[styles.smallActionText, { color: t.foreground }]}>
                  Reload Config
                </Text>
              </Pressable>
            </View>

            {mcpServers.map((server) => (
              <View
                key={server.name}
                style={[styles.subsection, { borderColor: t.border }]}
              >
                <View style={styles.subsectionHeader}>
                  <View style={styles.rowContent}>
                    <Text style={[styles.rowTitle, { color: t.foreground }]}>
                      {server.name}
                    </Text>
                    <Text
                      style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                    >
                      {server.server_type} • {server.tool_count} tools
                    </Text>
                  </View>
                  <Pill
                    label={server.connected ? "Connected" : server.status}
                    tone={
                      server.connected ? "success" : server.error ? "error" : "warning"
                    }
                  />
                </View>

                {server.tools.length ? (
                  <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
                    {server.tools.map((tool) => tool.name).join(", ")}
                  </Text>
                ) : null}
                {server.error ? (
                  <Text style={[styles.errorText, { color: t.error }]}>
                    {server.error}
                  </Text>
                ) : null}

                <View style={styles.actionsWrap}>
                  <Pressable
                    onPress={() => void handleToggleMcp(server)}
                    disabled={mcpBusyKey === server.name}
                    style={[styles.smallActionBtn, { borderColor: t.border }]}
                  >
                    {mcpBusyKey === server.name ? (
                      <ActivityIndicator color={t.userMessage} size="small" />
                    ) : (
                      <Text style={[styles.smallActionText, { color: t.userMessage }]}>
                        {server.connected ? "Disconnect" : "Connect"}
                      </Text>
                    )}
                  </Pressable>
                </View>
              </View>
            ))}
          </View>
        )}
      </GlassCard>

      <SectionTitle
        title="Skills"
        subtitle="Available global and project skills loaded by the server"
      />
      <GlassCard>
        {!isConnected ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            Connect to a server to inspect loaded skills.
          </Text>
        ) : skillsLoading ? (
          <View style={styles.loadingRow}>
            <ActivityIndicator color={t.userMessage} size="small" />
            <Text style={[styles.loadingText, { color: t.mutedForeground }]}>
              Loading skills…
            </Text>
          </View>
        ) : (
          <View style={styles.stack}>
            <MessageBanner text={skillsMessage} />
            {skills.length === 0 ? (
              <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
                No skills reported by the current server.
              </Text>
            ) : (
              skills.map((skill) => (
                <View
                  key={`${skill.source}:${skill.name}`}
                  style={[styles.subsection, { borderColor: t.border }]}
                >
                  <View style={styles.subsectionHeader}>
                    <View style={styles.rowContent}>
                      <Text style={[styles.rowTitle, { color: t.foreground }]}>
                        {skill.name}
                      </Text>
                      <Text
                        style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                      >
                        {skill.description}
                      </Text>
                    </View>
                    <Pill
                      label={skill.source}
                      tone={skill.source === "project" ? "info" : "neutral"}
                    />
                  </View>

                  <View style={styles.pillRow}>
                    {skill.version ? <Pill label={`v${skill.version}`} /> : null}
                    {skill.author ? <Pill label={skill.author} /> : null}
                    {skill.tags.slice(0, 4).map((tag) => (
                      <Pill key={tag} label={`#${tag}`} tone="info" />
                    ))}
                  </View>
                </View>
              ))
            )}
          </View>
        )}
      </GlassCard>

      <SectionTitle
        title="Preview & Ports"
        subtitle="Port forwarding behavior for web and desktop preview"
      />
      <GlassCard>
        {!isConnected ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            Connect to a server to manage preview settings.
          </Text>
        ) : previewLoading ? (
          <View style={styles.loadingRow}>
            <ActivityIndicator color={t.userMessage} size="small" />
            <Text style={[styles.loadingText, { color: t.mutedForeground }]}>
              Loading preview settings…
            </Text>
          </View>
        ) : !previewSettings ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            Preview settings are unavailable.
          </Text>
        ) : (
          <View style={styles.stack}>
            <MessageBanner
              text={previewMessage}
              tone={previewMessage?.includes("failed") ? "error" : "info"}
            />

            <View style={[styles.subsection, { borderColor: t.border }]}>
              <View style={styles.subsectionHeader}>
                <View style={styles.rowContent}>
                  <Text style={[styles.rowTitle, { color: t.foreground }]}>
                    Forwarding enabled
                  </Text>
                  <Text
                    style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                  >
                    Detect and proxy local development servers.
                  </Text>
                </View>
                <Pressable
                  onPress={() =>
                    void handleUpdatePreviewToggle({
                      enabled: !previewSettings.enabled,
                    })
                  }
                  disabled={previewBusyKey === "toggle"}
                  style={[
                    styles.toggle,
                    {
                      backgroundColor: previewSettings.enabled
                        ? t.userMessage
                        : t.border,
                    },
                  ]}
                >
                  <View
                    style={[
                      styles.toggleKnob,
                      {
                        alignSelf: previewSettings.enabled ? "flex-end" : "flex-start",
                      },
                    ]}
                  />
                </Pressable>
              </View>
            </View>

            <View style={[styles.subsection, { borderColor: t.border }]}>
              <View style={styles.subsectionHeader}>
                <View style={styles.rowContent}>
                  <Text style={[styles.rowTitle, { color: t.foreground }]}>
                    HTTP-like only
                  </Text>
                  <Text
                    style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                  >
                    Filter the preview list to ports that look like web servers.
                  </Text>
                </View>
                <Pressable
                  onPress={() =>
                    void handleUpdatePreviewToggle({
                      show_only_http_like: !previewSettings.show_only_http_like,
                    })
                  }
                  style={[
                    styles.toggle,
                    {
                      backgroundColor: previewSettings.show_only_http_like
                        ? t.userMessage
                        : t.border,
                    },
                  ]}
                >
                  <View
                    style={[
                      styles.toggleKnob,
                      {
                        alignSelf: previewSettings.show_only_http_like
                          ? "flex-end"
                          : "flex-start",
                      },
                    ]}
                  />
                </Pressable>
              </View>

              <View style={[styles.subsectionHeader, styles.compactHeader]}>
                <View style={styles.rowContent}>
                  <Text style={[styles.rowTitle, { color: t.foreground }]}>
                    Allow non-HTTP embed
                  </Text>
                  <Text
                    style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                  >
                    Keep non-HTTP listeners available to advanced preview flows.
                  </Text>
                </View>
                <Pressable
                  onPress={() =>
                    void handleUpdatePreviewToggle({
                      allow_force_open_non_http:
                        !previewSettings.allow_force_open_non_http,
                    })
                  }
                  style={[
                    styles.toggle,
                    {
                      backgroundColor: previewSettings.allow_force_open_non_http
                        ? t.userMessage
                        : t.border,
                    },
                  ]}
                >
                  <View
                    style={[
                      styles.toggleKnob,
                      {
                        alignSelf: previewSettings.allow_force_open_non_http
                          ? "flex-end"
                          : "flex-start",
                      },
                    ]}
                  />
                </Pressable>
              </View>
            </View>

            <View style={[styles.subsection, { borderColor: t.border }]}>
              <Text style={[styles.rowTitle, { color: t.foreground }]}>
                Probe timing
              </Text>
              <View style={styles.twoCol}>
                <View
                  style={[
                    styles.inputWrap,
                    {
                      flex: 1,
                      backgroundColor: g.background,
                      borderColor: g.border,
                    },
                  ]}
                >
                  <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>
                    AUTO REFRESH (s)
                  </Text>
                  <TextInput
                    style={[styles.input, { color: t.foreground }]}
                    value={previewDraft.autoRefreshSecs}
                    onChangeText={(value) =>
                      setPreviewDraft((current) => ({
                        ...current,
                        autoRefreshSecs: value,
                      }))
                    }
                    keyboardType="number-pad"
                  />
                </View>
                <View
                  style={[
                    styles.inputWrap,
                    {
                      flex: 1,
                      backgroundColor: g.background,
                      borderColor: g.border,
                    },
                  ]}
                >
                  <Text style={[styles.inputLabel, { color: t.mutedForeground }]}>
                    PROBE TIMEOUT (ms)
                  </Text>
                  <TextInput
                    style={[styles.input, { color: t.foreground }]}
                    value={previewDraft.probeTimeoutMs}
                    onChangeText={(value) =>
                      setPreviewDraft((current) => ({
                        ...current,
                        probeTimeoutMs: value,
                      }))
                    }
                    keyboardType="number-pad"
                  />
                </View>
              </View>

              <View style={styles.actionsWrap}>
                <Pressable
                  onPress={() => void handleSavePreviewNumbers()}
                  disabled={previewBusyKey === "numbers"}
                  style={[styles.smallActionBtn, { borderColor: t.border }]}
                >
                  <Text style={[styles.smallActionText, { color: t.userMessage }]}>
                    Save Timing
                  </Text>
                </Pressable>
                <Pressable
                  onPress={() => void loadPreview()}
                  style={[styles.smallActionBtn, { borderColor: t.border }]}
                >
                  <RefreshCw size={14} color={t.foreground} strokeWidth={1.8} />
                  <Text style={[styles.smallActionText, { color: t.foreground }]}>
                    Refresh Ports
                  </Text>
                </Pressable>
              </View>
            </View>

            <View style={[styles.subsection, { borderColor: t.border }]}>
              <Text style={[styles.rowTitle, { color: t.foreground }]}>Visible ports</Text>
              {previewPorts.length === 0 ? (
                <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
                  No forwardable ports are currently visible.
                </Text>
              ) : (
                previewPorts.map((port) => (
                  <View
                    key={port.port}
                    style={[styles.portRow, { borderColor: t.border }]}
                  >
                    <View style={styles.rowContent}>
                      <View style={styles.pillRow}>
                        <Text style={[styles.rowTitle, { color: t.foreground }]}>
                          {port.name}
                        </Text>
                        <Pill label={`:${port.port}`} tone="info" />
                        {port.pinned ? <Pill label="Pinned" tone="success" /> : null}
                      </View>
                      <Text
                        style={[styles.rowSubtitle, { color: t.mutedForeground }]}
                      >
                        {previewStatusText(port)}
                      </Text>
                    </View>

                    <View style={styles.actionsWrap}>
                      <Pressable
                        onPress={() => void handleTogglePinnedPort(port)}
                        disabled={previewBusyKey === `pin:${port.port}`}
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.userMessage }]}>
                          {port.pinned ? "Unpin" : "Pin"}
                        </Text>
                      </Pressable>
                      <Pressable
                        onPress={() => void handleHidePort(port)}
                        disabled={previewBusyKey === `hide:${port.port}`}
                        style={[styles.smallActionBtn, { borderColor: t.border }]}
                      >
                        <Text style={[styles.smallActionText, { color: t.warning }]}>
                          Hide
                        </Text>
                      </Pressable>
                    </View>
                  </View>
                ))
              )}
            </View>
          </View>
        )}
      </GlassCard>

      <SectionTitle title="Appearance" />
      <GlassCard>
        <View style={styles.schemeRow}>
          {schemeOptions.map((opt) => {
            const Icon = opt.icon;
            const selected = colorScheme === opt.key;
            return (
              <Pressable
                key={opt.key}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setColorScheme(opt.key);
                }}
                style={[
                  styles.schemeBtn,
                  {
                    backgroundColor: selected ? `${t.userMessage}20` : "transparent",
                    borderColor: selected ? t.userMessage : t.border,
                  },
                ]}
              >
                <Icon
                  size={18}
                  color={selected ? t.userMessage : t.mutedForeground}
                  strokeWidth={1.8}
                />
                <Text
                  style={[
                    styles.schemeBtnText,
                    { color: selected ? t.userMessage : t.mutedForeground },
                  ]}
                >
                  {opt.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      </GlassCard>

      <SectionTitle title="Notifications" />
      <GlassCard>
        <View style={styles.row}>
          <Bell size={20} color={t.mutedForeground} strokeWidth={1.8} />
          <View style={styles.rowContent}>
            <Text style={[styles.rowTitle, { color: t.foreground }]}>
              Delivery level
            </Text>
            <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
              Expo push token: {pushToken ? "registered" : "not registered"}
            </Text>
          </View>
        </View>

        <View style={[styles.separator, { backgroundColor: t.border }]} />

        <View style={styles.schemeRow}>
          {notifOptions.map((opt) => {
            const Icon = opt.icon;
            const selected = notificationLevel === opt.key;
            return (
              <Pressable
                key={opt.key}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  void changeNotificationLevel(opt.key);
                }}
                style={[
                  styles.schemeBtn,
                  {
                    backgroundColor: selected ? `${t.userMessage}20` : "transparent",
                    borderColor: selected ? t.userMessage : t.border,
                  },
                ]}
              >
                <Icon
                  size={18}
                  color={selected ? t.userMessage : t.mutedForeground}
                  strokeWidth={1.8}
                />
                <Text
                  style={[
                    styles.schemeBtnText,
                    { color: selected ? t.userMessage : t.mutedForeground },
                  ]}
                >
                  {opt.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      </GlassCard>

      <SectionTitle title="About" />
      <GlassCard>
        <View style={styles.row}>
          <Cpu size={20} color={t.mutedForeground} strokeWidth={1.8} />
          <View style={styles.rowContent}>
            <Text style={[styles.rowTitle, { color: t.foreground }]}>Krusty</Text>
            <Text style={[styles.rowSubtitle, { color: t.mutedForeground }]}>
              Expo mobile + web surface
            </Text>
          </View>
        </View>
      </GlassCard>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    padding: 20,
    paddingBottom: 48,
    gap: 14,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 4,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  closeBtn: {
    width: 34,
    height: 34,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
  },
  sectionHeader: {
    gap: 2,
    marginTop: 4,
  },
  sectionTitle: {
    fontSize: 14,
    fontWeight: "700",
    letterSpacing: 0.4,
    textTransform: "uppercase",
  },
  sectionSubtitle: {
    fontSize: 12,
    lineHeight: 18,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  rowContent: {
    flex: 1,
    gap: 2,
  },
  rowTitle: {
    fontSize: 16,
    fontWeight: "600",
  },
  rowSubtitle: {
    fontSize: 13,
    lineHeight: 18,
  },
  separator: {
    height: StyleSheet.hairlineWidth,
    marginVertical: 12,
  },
  actions: {
    flexDirection: "row",
    gap: 18,
    flexWrap: "wrap",
  },
  actionBtn: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  actionText: {
    fontSize: 15,
    fontWeight: "600",
  },
  connectForm: {
    gap: 14,
  },
  inputWrap: {
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 12,
  },
  inputLabel: {
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 0.5,
    marginBottom: 6,
  },
  input: {
    fontSize: 16,
    padding: 0,
  },
  inlineInput: {
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 12,
  },
  errorText: {
    fontSize: 13,
    lineHeight: 18,
  },
  connectBtn: {
    borderRadius: 16,
    paddingVertical: 14,
    alignItems: "center",
  },
  connectBtnText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "700",
  },
  schemeRow: {
    flexDirection: "row",
    gap: 10,
    flexWrap: "wrap",
  },
  schemeBtn: {
    minWidth: 110,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
  },
  schemeBtnText: {
    fontSize: 14,
    fontWeight: "600",
  },
  banner: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  bannerText: {
    fontSize: 13,
    lineHeight: 18,
  },
  loadingRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  loadingText: {
    fontSize: 14,
  },
  stack: {
    gap: 12,
  },
  subsection: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 16,
    padding: 14,
    gap: 10,
  },
  subsectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  compactHeader: {
    marginTop: 2,
  },
  pillRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexWrap: "wrap",
  },
  pill: {
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 999,
  },
  pillText: {
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 0.3,
  },
  actionsWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  smallActionBtn: {
    minHeight: 36,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  smallActionText: {
    fontSize: 13,
    fontWeight: "600",
  },
  oauthCodeHint: {
    fontSize: 15,
    fontWeight: "700",
  },
  twoCol: {
    flexDirection: "row",
    gap: 10,
  },
  toggle: {
    width: 48,
    borderRadius: 999,
    padding: 3,
  },
  toggleKnob: {
    width: 20,
    height: 20,
    borderRadius: 10,
    backgroundColor: "#fff",
  },
  portRow: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 12,
    gap: 8,
  },
  emptyText: {
    fontSize: 14,
    lineHeight: 20,
  },
});
