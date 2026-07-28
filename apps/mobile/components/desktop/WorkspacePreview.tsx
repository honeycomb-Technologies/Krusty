import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  Plus,
  RefreshCw,
  X,
} from "lucide-react-native";

import type { PortEntry, PreviewSettings } from "@krusty/api";

import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";

const PREVIEW_SESSION_KEY = "krusty_preview_tabs_v1";
// Preview content is untrusted localhost app output served through the Krusty proxy.
// Keep scripts/forms working for dev previews, but intentionally omit
// allow-same-origin and top-navigation so preview JavaScript cannot access the
// Krusty parent app or call privileged same-origin APIs.
const PREVIEW_IFRAME_SANDBOX =
  "allow-downloads allow-forms allow-modals allow-pointer-lock allow-popups allow-presentation allow-scripts";

type PersistedPreviewTab = {
  id: string;
  port: number | null;
  title: string;
  history: string[];
  historyIndex: number;
};

type PreviewTab = PersistedPreviewTab & {
  error: string | null;
  isLoading: boolean;
  reloadKey: number;
};

interface WorkspacePreviewProps {
  visible: boolean;
  style?: import("react-native").ViewStyle;
}

function createPreviewTabId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }

  return `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function createBlankPreviewTab(): PreviewTab {
  return {
    id: createPreviewTabId(),
    port: null,
    title: "New Tab",
    history: [],
    historyIndex: 0,
    error: null,
    isLoading: false,
    reloadKey: 0,
  };
}

function getPreviewUrl(serverUrl: string, port: number): string {
  return `${serverUrl.replace(/\/+$/, "")}/api/ports/${port}/proxy`;
}

function extractPortFromProxyPath(pathname: string): number | null {
  const match = pathname.match(/^\/api\/ports\/(\d+)\/proxy(?:\/.*)?$/);
  if (!match) return null;

  const port = Number(match[1]);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    return null;
  }

  return port;
}

function extractPortFromInput(raw: string, serverUrl: string): number | null {
  const input = raw.trim();
  if (!input) return null;

  const numeric = input.startsWith(":") ? input.slice(1) : input;
  if (/^\d+$/.test(numeric)) {
    const port = Number(numeric);
    return Number.isInteger(port) && port > 0 && port <= 65535 ? port : null;
  }

  try {
    const origin = new URL(serverUrl).origin;
    const parsed = new URL(input, origin);
    const proxyPort = extractPortFromProxyPath(parsed.pathname);
    if (proxyPort !== null) {
      return proxyPort;
    }

    if (parsed.port) {
      const port = Number(parsed.port);
      return Number.isInteger(port) && port > 0 && port <= 65535 ? port : null;
    }
  } catch {
    return null;
  }

  return null;
}

function syncTabTitlesWithPorts(tabs: PreviewTab[], ports: PortEntry[]): PreviewTab[] {
  let changed = false;
  const nextTabs = tabs.map((tab) => {
    if (tab.port === null) {
      return tab;
    }

    const match = ports.find((port) => port.port === tab.port);
    if (!match || match.name === tab.title) {
      return tab;
    }

    changed = true;
    return { ...tab, title: match.name };
  });

  return changed ? nextTabs : tabs;
}

function normalizeProxyUrl(
  raw: string,
  basePort: number,
  serverUrl: string,
  ports: PortEntry[],
  settings: PreviewSettings | null,
): string | null {
  const input = raw.trim();
  if (!input) return null;

  const origin = new URL(serverUrl).origin;
  const base = getPreviewUrl(serverUrl, basePort);

  let normalized: string;
  if (/^https?:\/\//i.test(input)) {
    normalized = input;
  } else if (input.startsWith("/api/ports/")) {
    normalized = `${origin}${input}`;
  } else if (input.startsWith("/")) {
    normalized = `${base}${input}`;
  } else if (input.startsWith("?") || input.startsWith("#")) {
    normalized = `${base}${input}`;
  } else {
    normalized = `${base}/${input}`;
  }

  let parsed: URL;
  try {
    parsed = new URL(normalized, origin);
  } catch {
    return null;
  }

  if (parsed.origin !== origin) {
    return null;
  }

  const targetPort = extractPortFromProxyPath(parsed.pathname);
  if (targetPort === null) {
    return null;
  }

  if (settings?.blocked_ports.includes(targetPort)) {
    return null;
  }

  if (!ports.some((port) => port.port === targetPort) && targetPort !== basePort) {
    return null;
  }

  return parsed.toString();
}

export function WorkspacePreview({ visible, style }: WorkspacePreviewProps) {
  const { theme } = useThemeContext();
  const { client, serverUrl } = useConnection();

  const [ports, setPorts] = useState<PortEntry[]>([]);
  const [settings, setSettings] = useState<PreviewSettings | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [discoveryWarning, setDiscoveryWarning] = useState<string | null>(null);
  const [previewTabs, setPreviewTabs] = useState<PreviewTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [previewAddress, setPreviewAddress] = useState("");
  const [tabsRestored, setTabsRestored] = useState(false);

  const isWeb = Platform.OS === "web";
  const t = theme.colors;
  const activeTab =
    previewTabs.find((tab) => tab.id === activeTabId) ?? null;

  useEffect(() => {
    const url =
      activeTab && activeTab.port !== null
        ? activeTab.history[activeTab.historyIndex] ?? ""
        : "";
    setPreviewAddress(url);
  }, [activeTab]);

  useEffect(() => {
    if (!isWeb || !serverUrl || tabsRestored) return;

    const raw = sessionStorage.getItem(PREVIEW_SESSION_KEY);
    if (!raw) {
      const blankTab = createBlankPreviewTab();
      setPreviewTabs([blankTab]);
      setActiveTabId(blankTab.id);
      setTabsRestored(true);
      return;
    }

    try {
      const parsed = JSON.parse(raw) as {
        tabs?: PersistedPreviewTab[];
        activeTabId?: string | null;
      };

      const restoredTabs: PreviewTab[] = Array.isArray(parsed.tabs)
        ? parsed.tabs
            .filter(
              (tab) =>
                tab &&
                typeof tab.id === "string" &&
                Array.isArray(tab.history) &&
                (
                  tab.port === null ||
                  (
                    Number.isInteger(tab.port) &&
                    tab.port > 0 &&
                    tab.port <= 65535 &&
                    tab.history.length > 0
                  )
                ),
            )
            .map((tab) => {
              if (tab.port === null) {
                return {
                  ...createBlankPreviewTab(),
                  id: tab.id,
                  title: tab.title || "New Tab",
                };
              }

              const restoredPort = tab.port;
              const safeHistory = tab.history
                .map((entry) => normalizeProxyUrl(entry, restoredPort, serverUrl, [], null))
                .filter((entry): entry is string => Boolean(entry));

              if (safeHistory.length === 0) {
                safeHistory.push(getPreviewUrl(serverUrl, restoredPort));
              }

              const boundedIndex = Math.min(
                Math.max(0, Number(tab.historyIndex) || 0),
                safeHistory.length - 1,
              );

              return {
                id: tab.id,
                port: restoredPort,
                title: tab.title || `Port ${restoredPort}`,
                history: safeHistory,
                historyIndex: boundedIndex,
                error: null,
                isLoading: false,
                reloadKey: 0,
              };
            })
        : [];

      setPreviewTabs(restoredTabs);
      if (restoredTabs.length === 0) {
        const blankTab = createBlankPreviewTab();
        setPreviewTabs([blankTab]);
        setActiveTabId(blankTab.id);
      } else if (
        parsed.activeTabId &&
        restoredTabs.some((tab) => tab.id === parsed.activeTabId)
      ) {
        setActiveTabId(parsed.activeTabId);
      } else {
        setActiveTabId(restoredTabs[0]?.id ?? null);
      }
    } catch {
      sessionStorage.removeItem(PREVIEW_SESSION_KEY);
      const blankTab = createBlankPreviewTab();
      setPreviewTabs([blankTab]);
      setActiveTabId(blankTab.id);
    } finally {
      setTabsRestored(true);
    }
  }, [isWeb, serverUrl, tabsRestored]);

  useEffect(() => {
    if (!isWeb) return;

    const payload = JSON.stringify({
      tabs: previewTabs.map((tab) => ({
        id: tab.id,
        port: tab.port,
        title: tab.title,
        history: tab.history,
        historyIndex: tab.historyIndex,
      })),
      activeTabId,
    });
    sessionStorage.setItem(PREVIEW_SESSION_KEY, payload);
  }, [activeTabId, isWeb, previewTabs]);

  const createNewTab = useCallback(() => {
    const tab = createBlankPreviewTab();
    setPreviewTabs((current) => [...current, tab]);
    setActiveTabId(tab.id);
    setError(null);
  }, []);

  const loadPorts = useCallback(
    async (background = false) => {
      if (!client) return;

      if (background) {
        setRefreshing(true);
      } else {
        setLoading(true);
      }

      try {
        const response = await client.getPorts();
        setPorts(response.ports);
        setSettings(response.settings);
        setDiscoveryWarning(response.discovery_error ?? null);
        setError(null);
        setPreviewTabs((current) => syncTabTitlesWithPorts(current, response.ports));
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load preview ports.");
      } finally {
        setLoading(false);
        setRefreshing(false);
      }
    },
    [client],
  );

  useEffect(() => {
    if (!isWeb || !visible || !client) return;

    void loadPorts();
  }, [client, isWeb, loadPorts, visible]);

  useEffect(() => {
    if (!isWeb || !visible || !client) return;

    const intervalMs = Math.max(2, settings?.auto_refresh_secs ?? 5) * 1000;
    const timer = setInterval(() => {
      void loadPorts(true);
    }, intervalMs);

    return () => clearInterval(timer);
  }, [client, isWeb, loadPorts, settings?.auto_refresh_secs, visible]);

  const openPortInPreview = useCallback(
    (port: PortEntry) => {
      if (!serverUrl) return;

      if (!port.active) {
        setError(`Port ${port.port} is not currently active.`);
        return;
      }

      if (
        !port.is_previewable_http &&
        !(settings?.allow_force_open_non_http ?? false)
      ) {
        setError(`Port ${port.port} is listening but did not pass the HTTP probe.`);
        return;
      }

      if (activeTab?.port === null) {
        setPreviewTabs((current) =>
          current.map((tab) =>
            tab.id === activeTab.id
              ? {
                  ...tab,
                  port: port.port,
                  title: port.name,
                  history: [getPreviewUrl(serverUrl, port.port)],
                  historyIndex: 0,
                  error: null,
                  isLoading: true,
                  reloadKey: 0,
                }
              : tab,
          ),
        );
        setActiveTabId(activeTab.id);
        setError(null);
        return;
      }

      const existing = previewTabs.find((tab) => tab.port === port.port);
      if (existing) {
        setActiveTabId(existing.id);
        return;
      }

      const nextTab: PreviewTab = {
        id: createPreviewTabId(),
        port: port.port,
        title: port.name,
        history: [getPreviewUrl(serverUrl, port.port)],
        historyIndex: 0,
        error: null,
        isLoading: true,
        reloadKey: 0,
      };
      setPreviewTabs((current) => [...current, nextTab]);
      setActiveTabId(nextTab.id);
      setError(null);
    },
    [activeTab, previewTabs, serverUrl, settings?.allow_force_open_non_http],
  );

  const closeTab = useCallback(
    (tabId: string) => {
      setPreviewTabs((current) => {
        const index = current.findIndex((tab) => tab.id === tabId);
        if (index === -1) {
          return current;
        }

        const nextTabs = current.filter((tab) => tab.id !== tabId);
        if (nextTabs.length === 0) {
          const blankTab = createBlankPreviewTab();
          setActiveTabId(blankTab.id);
          return [blankTab];
        }

        setActiveTabId((currentActive) => {
          if (currentActive !== tabId) {
            return currentActive;
          }

          const nextActive =
            nextTabs[Math.max(0, index - 1)]?.id ?? nextTabs[0]?.id ?? null;
          return nextActive;
        });

        return nextTabs;
      });
    },
    [],
  );

  const updateTabNavigation = useCallback(
    (tabId: string, rawUrl: string, pushHistory: boolean) => {
      if (!serverUrl) return;

      const current = previewTabs.find((tab) => tab.id === tabId);
      if (!current) return;
      if (current.port === null) {
        setError("Select an available preview port.");
        return;
      }

      const normalized = normalizeProxyUrl(
        rawUrl,
        current.port,
        serverUrl,
        ports,
        settings,
      );
      if (!normalized) {
        setError("Invalid preview URL. Use a current /api/ports/{port}/proxy path.");
        return;
      }

      const parsed = new URL(normalized);
      const targetPort = extractPortFromProxyPath(parsed.pathname);
      if (targetPort === null) {
        setError("Invalid preview URL path.");
        return;
      }

      const matchedPort = ports.find((port) => port.port === targetPort);
      if (!matchedPort && targetPort !== current.port) {
        setError(`Port ${targetPort} is not available in the current preview list.`);
        return;
      }

      setPreviewTabs((tabs) =>
        tabs.map((tab) => {
          if (tab.id !== tabId) {
            return tab;
          }

          let history = tab.history;
          let historyIndex = tab.historyIndex;
          if (pushHistory) {
            history = [...tab.history.slice(0, historyIndex + 1), normalized];
            historyIndex = history.length - 1;
          }

          return {
            ...tab,
            port: targetPort,
            title: matchedPort?.name ?? `Port ${targetPort}`,
            history,
            historyIndex,
            isLoading: true,
            error: null,
          };
        }),
      );
      setActiveTabId(tabId);
      setError(null);
    },
    [ports, previewTabs, serverUrl, settings],
  );

  const availablePorts = useMemo(
    () =>
      ports.filter(
        (port) => port.active && port.is_previewable_http,
      ),
    [ports],
  );
  const showPortLauncher = !activeTab || activeTab.port === null;

  const handleAddressSubmit = useCallback(() => {
    if (!serverUrl) return;

    if (showPortLauncher) {
      const port = extractPortFromInput(previewAddress, serverUrl);
      const match = port
        ? availablePorts.find((candidate) => candidate.port === port)
        : null;
      if (!match) {
        setError("Select an available preview port.");
        return;
      }

      openPortInPreview(match);
      return;
    }

    if (activeTab) {
      updateTabNavigation(activeTab.id, previewAddress, true);
      return;
    }

    const port = extractPortFromInput(previewAddress, serverUrl);
    const match = port
      ? availablePorts.find((candidate) => candidate.port === port)
      : null;
    if (!match) {
      setError("Select an available preview port.");
      return;
    }

    openPortInPreview(match);
  }, [
    activeTab,
    availablePorts,
    openPortInPreview,
    previewAddress,
    serverUrl,
    showPortLauncher,
    updateTabNavigation,
  ]);

  if (!isWeb) {
    return null;
  }

  // Keep the browser pane mounted across toolbox tab switches so previews
  // do not reload every time Browser is re-selected.
  return (
    <View
      pointerEvents={visible ? 'auto' : 'none'}
      style={[
        styles.container,
        { backgroundColor: t.background, borderTopColor: t.border },
        !visible && styles.hiddenSurface,
        style,
      ]}
    >
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        {previewTabs.map((tab) => {
          const active = activeTabId === tab.id;
          return (
            <View
              key={tab.id}
              style={[
                styles.tab,
                {
                  backgroundColor: active ? `${t.userMessage}18` : "transparent",
                  borderColor: active ? `${t.userMessage}25` : "transparent",
                },
              ]}
            >
              <Pressable
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setActiveTabId(tab.id);
                }}
                style={styles.tabPressable}
              >
                <Text
                  style={[
                    styles.tabLabel,
                    { color: active ? t.foreground : t.mutedForeground },
                  ]}
                  numberOfLines={1}
                >
                  {tab.title}
                </Text>
              </Pressable>
              <Pressable
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  closeTab(tab.id);
                }}
                hitSlop={8}
              >
                <X size={12} color={t.mutedForeground} strokeWidth={2} />
              </Pressable>
            </View>
          );
        })}

        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            createNewTab();
          }}
          style={styles.iconBtn}
        >
          <Plus size={16} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>

        <View style={styles.grow} />

        {refreshing ? <ActivityIndicator color={t.userMessage} size="small" /> : null}
      </View>

      {error ? (
        <View style={[styles.banner, { backgroundColor: `${t.error}10`, borderColor: `${t.error}20` }]}>
          <AlertCircle size={14} color={t.error} strokeWidth={1.8} />
          <Text style={[styles.bannerText, { color: t.error }]}>{error}</Text>
        </View>
      ) : null}

      {discoveryWarning ? (
        <View
          style={[
            styles.banner,
            { backgroundColor: `${t.warning}10`, borderColor: `${t.warning}20` },
          ]}
        >
          <AlertCircle size={14} color={t.warning} strokeWidth={1.8} />
          <Text style={[styles.bannerText, { color: t.warning }]}>
            {discoveryWarning}
          </Text>
        </View>
      ) : null}

      <View style={styles.browserPane}>
        <View style={[styles.browserToolbar, { borderBottomColor: t.border }]}>
          <Pressable
            onPress={() => {
              if (!activeTab || activeTab.historyIndex === 0) return;
              setPreviewTabs((tabs) =>
                tabs.map((tab) =>
                  tab.id === activeTab.id
                    ? {
                        ...tab,
                        historyIndex: tab.historyIndex - 1,
                        isLoading: true,
                        error: null,
                      }
                    : tab,
                ),
              );
            }}
            disabled={!activeTab || activeTab.historyIndex === 0}
            style={[styles.iconBtn, styles.toolbarBtn]}
          >
            <ArrowLeft
              size={15}
              color={!activeTab || activeTab.historyIndex === 0 ? t.border : t.mutedForeground}
              strokeWidth={1.8}
            />
          </Pressable>

          <Pressable
            onPress={() => {
              if (!activeTab || activeTab.historyIndex >= activeTab.history.length - 1) return;
              setPreviewTabs((tabs) =>
                tabs.map((tab) =>
                  tab.id === activeTab.id
                    ? {
                        ...tab,
                        historyIndex: tab.historyIndex + 1,
                        isLoading: true,
                        error: null,
                      }
                    : tab,
                ),
              );
            }}
            disabled={!activeTab || activeTab.historyIndex >= activeTab.history.length - 1}
            style={[styles.iconBtn, styles.toolbarBtn]}
          >
            <ArrowRight
              size={15}
              color={
                !activeTab || activeTab.historyIndex >= activeTab.history.length - 1
                  ? t.border
                  : t.mutedForeground
              }
              strokeWidth={1.8}
            />
          </Pressable>

          <Pressable
            onPress={() => {
              if (!activeTab || activeTab.port === null) return;
              setPreviewTabs((tabs) =>
                tabs.map((tab) =>
                  tab.id === activeTab.id
                    ? {
                        ...tab,
                        isLoading: true,
                        error: null,
                        reloadKey: tab.reloadKey + 1,
                      }
                    : tab,
                ),
              );
            }}
            disabled={!activeTab || activeTab.port === null}
            style={[styles.iconBtn, styles.toolbarBtn]}
          >
            <RefreshCw
              size={15}
              color={activeTab?.port !== null ? t.mutedForeground : t.border}
              strokeWidth={1.8}
            />
          </Pressable>

          <TextInput
            value={previewAddress}
            onChangeText={setPreviewAddress}
            onSubmitEditing={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              handleAddressSubmit();
            }}
            style={[
              styles.addressInput,
              {
                color: t.foreground,
                borderColor: t.border,
                backgroundColor: t.card,
              },
            ]}
            placeholder={showPortLauncher ? "Select a port or enter :5173" : "Preview URL"}
            placeholderTextColor={`${t.mutedForeground}88`}
            autoCapitalize="none"
            autoCorrect={false}
          />
        </View>

        <View style={styles.previewArea}>
          {showPortLauncher ? (
            <View style={styles.portLauncherPage}>
              {loading && availablePorts.length === 0 ? (
                <View style={styles.inlineState}>
                  <ActivityIndicator color={t.userMessage} size="small" />
                  <Text style={[styles.stateText, { color: t.mutedForeground }]}>
                    Looking for previewable ports...
                  </Text>
                </View>
              ) : availablePorts.length === 0 ? (
                <Text style={[styles.stateText, { color: t.mutedForeground }]}>
                  Waiting for previewable ports.
                </Text>
              ) : (
                <View style={styles.portStack}>
                  {availablePorts.map((port) => (
                    <Pressable
                      key={port.port}
                      onPress={() => {
                        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                        openPortInPreview(port);
                      }}
                      style={[
                        styles.portButton,
                        {
                          backgroundColor: t.card,
                          borderColor: t.border,
                        },
                      ]}
                    >
                      <Text style={[styles.portButtonText, { color: t.foreground }]}>
                        :{port.port}
                      </Text>
                    </Pressable>
                  ))}
                </View>
              )}
            </View>
          ) : (
            previewTabs
              .filter((tab) => tab.port !== null)
              .map((tab) => {
              const active = activeTabId === tab.id;
              return (
                <View
                  key={tab.id}
                  style={[styles.previewFrameWrap, !active && styles.hiddenFrame]}
                >
                {tab.isLoading ? (
                  <View style={styles.previewOverlay}>
                    <ActivityIndicator color={t.userMessage} size="small" />
                    <Text style={[styles.stateText, { color: t.mutedForeground }]}>
                      Loading preview…
                    </Text>
                  </View>
                ) : null}

                {tab.error ? (
                  <View style={styles.previewOverlay}>
                    <Text style={[styles.stateText, { color: t.error }]}>
                      {tab.error}
                    </Text>
                  </View>
                ) : null}

                <div style={{ width: "100%", height: "100%" }}>
                    <iframe
                      key={`${tab.id}:${tab.historyIndex}:${tab.reloadKey}`}
                      src={tab.history[tab.historyIndex]}
                      sandbox={PREVIEW_IFRAME_SANDBOX}
                      style={{ width: "100%", height: "100%", border: "none" }}
                      title={tab.title}
                      onLoad={() => {
                      setPreviewTabs((tabs) =>
                        tabs.map((entry) =>
                          entry.id === tab.id
                            ? { ...entry, isLoading: false, error: null }
                            : entry,
                        ),
                      );
                    }}
                    onError={() => {
                      setPreviewTabs((tabs) =>
                        tabs.map((entry) =>
                          entry.id === tab.id
                            ? {
                                ...entry,
                                isLoading: false,
                                error: "Preview failed to render in embedded mode.",
                              }
                            : entry,
                        ),
                      );
                    }}
                  />
                </div>
              </View>
              );
            })
          )}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    height: 380,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  hiddenSurface: {
    opacity: 0,
  },
  tabBar: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  tab: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingLeft: 10,
    paddingRight: 8,
    paddingVertical: 6,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    maxWidth: 180,
  },
  tabPressable: {
    flexShrink: 1,
  },
  tabLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  iconBtn: {
    padding: 6,
    borderRadius: 8,
  },
  grow: {
    flex: 1,
  },
  banner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  bannerText: {
    flex: 1,
    fontSize: 12,
    lineHeight: 18,
  },
  browserPane: {
    flex: 1,
  },
  browserToolbar: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  toolbarBtn: {
    borderWidth: StyleSheet.hairlineWidth,
  },
  addressInput: {
    flex: 1,
    minHeight: 36,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    fontSize: 13,
  },
  inlineState: {
    minHeight: 38,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  stateText: {
    fontSize: 13,
  },
  portLauncherPage: {
    ...StyleSheet.absoluteFillObject,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
  portStack: {
    width: "100%",
    maxWidth: 220,
    gap: 10,
  },
  portButton: {
    minHeight: 44,
    paddingHorizontal: 18,
    paddingVertical: 11,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
  },
  portButtonText: {
    fontSize: 15,
    fontWeight: "700",
  },
  previewArea: {
    flex: 1,
    position: "relative",
  },
  previewFrameWrap: {
    ...StyleSheet.absoluteFillObject,
  },
  hiddenFrame: {
    display: "none",
  },
  previewOverlay: {
    position: "absolute",
    top: 16,
    left: 16,
    zIndex: 2,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
    borderRadius: 10,
    backgroundColor: "rgba(10,10,10,0.6)",
  },
});
