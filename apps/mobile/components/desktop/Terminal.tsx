import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  AlertCircle,
  Plus,
  RefreshCw,
  TerminalSquare,
  X,
} from "lucide-react-native";

import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useWorkspaceStore } from "../../hooks/useStores";
import { useThemeContext } from "../../hooks/useTheme";

const MAX_RECONNECT_ATTEMPTS = 8;
const RECONNECT_INITIAL_DELAY_MS = 250;
const RECONNECT_MAX_DELAY_MS = 5_000;
const RECONNECT_STABLE_RESET_MS = 30_000;
const HEARTBEAT_INTERVAL_MS = 15_000;
const HEARTBEAT_TIMEOUT_MS = 45_000;

interface TerminalTab {
  id: string;
  label: string;
  connected: boolean;
  error: string | null;
}

interface TerminalProps {
  visible: boolean;
}

function createTerminalId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }

  return `term-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function createXtermTheme(colors: ReturnType<typeof useThemeContext>["theme"]["colors"]) {
  return {
    background: colors.background,
    foreground: colors.foreground,
    cursor: colors.userMessage,
    cursorAccent: colors.background,
    selectionBackground: `${colors.userMessage}35`,
    black: "#18181b",
    red: "#ef4444",
    green: "#22c55e",
    yellow: "#eab308",
    blue: "#3b82f6",
    magenta: "#a855f7",
    cyan: "#06b6d4",
    white: "#fafafa",
    brightBlack: "#52525b",
    brightRed: "#f87171",
    brightGreen: "#4ade80",
    brightYellow: "#facc15",
    brightBlue: "#60a5fa",
    brightMagenta: "#c084fc",
    brightCyan: "#22d3ee",
    brightWhite: "#ffffff",
  };
}

function escapeShellPath(path: string): string {
  return path.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

export function Terminal({ visible }: TerminalProps) {
  const { theme } = useThemeContext();
  const { serverUrl } = useConnection();
  const workspaceDirectory = useWorkspaceStore((state) => state.directory) ?? null;

  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  const isWeb = Platform.OS === "web";
  const t = theme.colors;

  const createTab = useCallback(() => {
    const id = createTerminalId();
    setTabs((current) => [
      ...current,
      {
        id,
        label: `Terminal ${current.length + 1}`,
        connected: false,
        error: null,
      },
    ]);
    setActiveTabId(id);
  }, []);

  const closeTab = useCallback(
    (tabId: string) => {
      setTabs((current) => {
        const index = current.findIndex((tab) => tab.id === tabId);
        if (index === -1) {
          return current;
        }

        const nextTabs = current.filter((tab) => tab.id !== tabId);
        setActiveTabId((currentActive) =>
          currentActive === tabId
            ? nextTabs[Math.max(0, index - 1)]?.id ?? nextTabs[0]?.id ?? null
            : currentActive,
        );
        return nextTabs;
      });
    },
    [],
  );

  const handleStatusChange = useCallback(
    (tabId: string, patch: Partial<Pick<TerminalTab, "connected" | "error">>) => {
      setTabs((current) =>
        current.map((tab) => (tab.id === tabId ? { ...tab, ...patch } : tab)),
      );
    },
    [],
  );

  useEffect(() => {
    if (!isWeb || !visible || !serverUrl || tabs.length > 0) return;
    createTab();
  }, [createTab, isWeb, serverUrl, tabs.length, visible]);

  if (!isWeb || !visible) {
    return null;
  }

  return (
    <View style={[styles.container, { backgroundColor: t.background, borderTopColor: t.border }]}>
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        <TerminalSquare size={16} color={t.mutedForeground} strokeWidth={1.8} />

        {tabs.map((tab) => {
          const active = activeTabId === tab.id;
          const statusColor = tab.connected
            ? t.success
            : tab.error && !tab.error.startsWith("Reconnecting")
              ? t.error
              : t.warning;

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
                <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
                <Text
                  style={[
                    styles.tabLabel,
                    { color: active ? t.foreground : t.mutedForeground },
                  ]}
                  numberOfLines={1}
                >
                  {tab.label}
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
            createTab();
          }}
          style={styles.iconBtn}
        >
          <Plus size={16} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>
      </View>

      <View style={styles.terminalArea}>
        {!serverUrl ? (
          <View style={styles.centerState}>
            <Text style={[styles.stateText, { color: t.mutedForeground }]}>
              Connect to a server to open a terminal.
            </Text>
          </View>
        ) : tabs.length === 0 ? (
          <View style={styles.centerState}>
            <Text style={[styles.stateText, { color: t.mutedForeground }]}>
              No terminal open.
            </Text>
          </View>
        ) : (
          tabs.map((tab) => (
            <TerminalPane
              key={tab.id}
              active={activeTabId === tab.id}
              serverUrl={serverUrl}
              tab={tab}
              themeColors={theme.colors}
              workspaceDirectory={workspaceDirectory}
              onStatusChange={handleStatusChange}
            />
          ))
        )}
      </View>
    </View>
  );
}

function TerminalPane({
  active,
  serverUrl,
  tab,
  themeColors,
  workspaceDirectory,
  onStatusChange,
}: {
  active: boolean;
  serverUrl: string;
  tab: TerminalTab;
  themeColors: ReturnType<typeof useThemeContext>["theme"]["colors"];
  workspaceDirectory: string | null;
  onStatusChange: (
    tabId: string,
    patch: Partial<Pick<TerminalTab, "connected" | "error">>,
  ) => void;
}) {
  const divRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<any>(null);
  const fitAddonRef = useRef<any>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const serverUrlRef = useRef(serverUrl);
  const activeRef = useRef(active);
  const workspaceDirectoryRef = useRef(workspaceDirectory);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stableResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const heartbeatIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const heartbeatTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const resizeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flushFrameRef = useRef<number | null>(null);
  const pendingOutputRef = useRef("");
  const lastColsRef = useRef(0);
  const lastRowsRef = useRef(0);
  const lastSyncedDirectoryRef = useRef<string | null>(null);
  const manualDisconnectRef = useRef(false);

  const queueOutput = useCallback((data: string) => {
    pendingOutputRef.current += data;
    if (flushFrameRef.current !== null) {
      return;
    }

    flushFrameRef.current = window.requestAnimationFrame(() => {
      flushFrameRef.current = null;
      if (terminalRef.current && pendingOutputRef.current.length > 0) {
        terminalRef.current.write(pendingOutputRef.current);
        pendingOutputRef.current = "";
      }
    });
  }, []);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const clearStableResetTimer = useCallback(() => {
    if (stableResetTimerRef.current) {
      clearTimeout(stableResetTimerRef.current);
      stableResetTimerRef.current = null;
    }
  }, []);

  const clearHeartbeat = useCallback(() => {
    if (heartbeatIntervalRef.current) {
      clearInterval(heartbeatIntervalRef.current);
      heartbeatIntervalRef.current = null;
    }
    if (heartbeatTimeoutRef.current) {
      clearTimeout(heartbeatTimeoutRef.current);
      heartbeatTimeoutRef.current = null;
    }
  }, []);

  const touchHeartbeat = useCallback(
    (ws: WebSocket) => {
      if (heartbeatTimeoutRef.current) {
        clearTimeout(heartbeatTimeoutRef.current);
      }

      heartbeatTimeoutRef.current = setTimeout(() => {
        if (wsRef.current === ws && ws.readyState === WebSocket.OPEN) {
          ws.close(4000, "heartbeat_timeout");
        }
      }, HEARTBEAT_TIMEOUT_MS);
    },
    [],
  );

  const fitAndResize = useCallback((force = false) => {
    if (!terminalRef.current || !fitAddonRef.current) return;

    fitAddonRef.current.fit();
    const cols = terminalRef.current.cols;
    const rows = terminalRef.current.rows;
    if (!force && cols === lastColsRef.current && rows === lastRowsRef.current) {
      return;
    }

    lastColsRef.current = cols;
    lastRowsRef.current = rows;

    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resize", cols, rows }));
    }
  }, []);

  const syncWorkspaceDirectory = useCallback(() => {
    const directory = workspaceDirectoryRef.current;
    const ws = wsRef.current;

    if (
      !activeRef.current ||
      !directory ||
      !ws ||
      ws.readyState !== WebSocket.OPEN ||
      lastSyncedDirectoryRef.current === directory
    ) {
      return;
    }

    ws.send(
      JSON.stringify({
        type: "input",
        data: `cd "${escapeShellPath(directory)}"\n`,
      }),
    );
    lastSyncedDirectoryRef.current = directory;
  }, []);

  const connectToTerminal = useCallback(
    (isReconnect: boolean) => {
      if (!terminalRef.current || !serverUrlRef.current) return;

      manualDisconnectRef.current = false;
      clearReconnectTimer();

      const existing = wsRef.current;
      if (
        existing?.readyState === WebSocket.OPEN ||
        existing?.readyState === WebSocket.CONNECTING
      ) {
        return;
      }

      if (existing && existing.readyState !== WebSocket.CLOSED) {
        existing.close();
      }

      clearHeartbeat();
      wsRef.current = null;

      if (!isReconnect) {
        reconnectAttemptsRef.current = 0;
        onStatusChange(tab.id, { connected: false, error: null });
      }

      const wsUrl = serverUrlRef.current.replace(/^http/i, "ws") + "/ws/terminal";
      const ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";
      wsRef.current = ws;

      ws.onopen = () => {
        onStatusChange(tab.id, { connected: true, error: null });
        clearStableResetTimer();
        stableResetTimerRef.current = setTimeout(() => {
          if (wsRef.current === ws && ws.readyState === WebSocket.OPEN) {
            reconnectAttemptsRef.current = 0;
          }
          stableResetTimerRef.current = null;
        }, RECONNECT_STABLE_RESET_MS);

        clearHeartbeat();
        touchHeartbeat(ws);
        heartbeatIntervalRef.current = setInterval(() => {
          if (wsRef.current !== ws || ws.readyState !== WebSocket.OPEN) {
            return;
          }

          try {
            ws.send(JSON.stringify({ type: "ping" }));
          } catch {
            ws.close();
          }
        }, HEARTBEAT_INTERVAL_MS);

        try {
          ws.send(JSON.stringify({ type: "hello", binary_output: true }));
        } catch {
          ws.close();
          return;
        }

        window.requestAnimationFrame(() => {
          fitAndResize(true);
          terminalRef.current?.focus?.();
          syncWorkspaceDirectory();
        });
      };

      ws.onmessage = (event) => {
        touchHeartbeat(ws);

        if (event.data instanceof ArrayBuffer) {
          queueOutput(new TextDecoder().decode(new Uint8Array(event.data)));
          return;
        }

        if (event.data instanceof Blob) {
          void event.data.arrayBuffer().then((buffer) => {
            queueOutput(new TextDecoder().decode(new Uint8Array(buffer)));
          });
          return;
        }

        const raw = typeof event.data === "string" ? event.data : String(event.data);
        try {
          const message = JSON.parse(raw);
          if (message.type === "output" && typeof message.data === "string") {
            queueOutput(message.data);
            return;
          }
          if (message.type === "error" && typeof message.error === "string") {
            onStatusChange(tab.id, { connected: false, error: message.error });
            return;
          }
        } catch {
          queueOutput(raw);
          return;
        }
      };

      ws.onerror = () => {
        onStatusChange(tab.id, { connected: false, error: "WebSocket error" });
      };

      ws.onclose = () => {
        clearHeartbeat();
        clearStableResetTimer();
        if (wsRef.current === ws) {
          wsRef.current = null;
        }

        onStatusChange(tab.id, { connected: false });

        if (manualDisconnectRef.current) {
          manualDisconnectRef.current = false;
          return;
        }

        if (reconnectTimerRef.current) {
          return;
        }

        const attempt = reconnectAttemptsRef.current;
        if (attempt >= MAX_RECONNECT_ATTEMPTS) {
          onStatusChange(tab.id, {
            connected: false,
            error: "Connection lost. Retry limit reached.",
          });
          return;
        }

        const attemptNumber = attempt + 1;
        reconnectAttemptsRef.current = attemptNumber;
        onStatusChange(tab.id, {
          connected: false,
          error: `Reconnecting (${attemptNumber}/${MAX_RECONNECT_ATTEMPTS})…`,
        });

        const delay = Math.min(
          RECONNECT_INITIAL_DELAY_MS * 2 ** attempt,
          RECONNECT_MAX_DELAY_MS,
        );
        reconnectTimerRef.current = setTimeout(() => {
          reconnectTimerRef.current = null;
          connectToTerminal(true);
        }, delay);
      };
    },
    [
      clearHeartbeat,
      clearReconnectTimer,
      clearStableResetTimer,
      fitAndResize,
      onStatusChange,
      queueOutput,
      syncWorkspaceDirectory,
      tab.id,
      touchHeartbeat,
    ],
  );

  useEffect(() => {
    activeRef.current = active;
    if (active && terminalRef.current && fitAddonRef.current) {
      window.requestAnimationFrame(() => {
        if (pendingOutputRef.current.length > 0) {
          terminalRef.current.write(pendingOutputRef.current);
          pendingOutputRef.current = "";
        }
        fitAndResize(true);
        terminalRef.current?.focus?.();
        syncWorkspaceDirectory();
      });
    }
  }, [active, fitAndResize, syncWorkspaceDirectory]);

  useEffect(() => {
    workspaceDirectoryRef.current = workspaceDirectory;
    syncWorkspaceDirectory();
  }, [syncWorkspaceDirectory, workspaceDirectory]);

  useEffect(() => {
    serverUrlRef.current = serverUrl;
  }, [serverUrl]);

  useEffect(() => {
    if (Platform.OS !== "web" || !divRef.current) {
      return;
    }

    let cancelled = false;

    void (async () => {
      try {
        const { Terminal } = await import("xterm");
        const { FitAddon } = await import("@xterm/addon-fit");
        await import("xterm/css/xterm.css");

        if (cancelled || !divRef.current) {
          return;
        }

        const terminal = new Terminal({
          cursorBlink: true,
          cursorStyle: "block",
          fontSize: 14,
          fontFamily: "JetBrains Mono, Fira Code, monospace",
          theme: createXtermTheme(themeColors),
        });

        const fitAddon = new FitAddon();
        terminal.loadAddon(fitAddon);
        terminal.open(divRef.current);

        terminalRef.current = terminal;
        fitAddonRef.current = fitAddon;

        terminal.onData((data: string) => {
          const ws = wsRef.current;
          if (ws?.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: "input", data }));
          }
        });

        resizeObserverRef.current = new ResizeObserver(() => {
          if (!activeRef.current) return;

          if (resizeTimeoutRef.current) {
            clearTimeout(resizeTimeoutRef.current);
          }
          resizeTimeoutRef.current = setTimeout(() => fitAndResize(), 75);
        });
        resizeObserverRef.current.observe(divRef.current);

        window.requestAnimationFrame(() => {
          fitAndResize(true);
          connectToTerminal(false);
        });
      } catch (err) {
        onStatusChange(tab.id, {
          connected: false,
          error: err instanceof Error ? err.message : "Failed to initialize terminal.",
        });
      }
    })();

    return () => {
      cancelled = true;
      manualDisconnectRef.current = true;
      clearReconnectTimer();
      clearStableResetTimer();
      clearHeartbeat();
      resizeObserverRef.current?.disconnect();
      resizeObserverRef.current = null;

      if (resizeTimeoutRef.current) {
        clearTimeout(resizeTimeoutRef.current);
        resizeTimeoutRef.current = null;
      }
      if (flushFrameRef.current !== null) {
        cancelAnimationFrame(flushFrameRef.current);
        flushFrameRef.current = null;
      }

      wsRef.current?.close();
      wsRef.current = null;
      fitAddonRef.current = null;
      terminalRef.current?.dispose();
      terminalRef.current = null;
      pendingOutputRef.current = "";
      lastSyncedDirectoryRef.current = null;
    };
  }, [
    clearHeartbeat,
    clearReconnectTimer,
    clearStableResetTimer,
    connectToTerminal,
    fitAndResize,
    onStatusChange,
    tab.id,
  ]);

  useEffect(() => {
    if (!terminalRef.current) return;
    terminalRef.current.setOption("theme", createXtermTheme(themeColors));
  }, [themeColors]);

  return (
    <View style={[styles.terminalPane, !active && styles.hiddenPane]}>
      <Pressable
        style={styles.terminalSurface}
        onPress={() => terminalRef.current?.focus?.()}
      >
        <div ref={divRef as any} style={{ width: "100%", height: "100%" }} />
      </Pressable>

      {!tab.connected ? (
        <View style={styles.overlay}>
          {tab.error && !tab.error.startsWith("Reconnecting") ? (
            <>
              <AlertCircle size={16} color={themeColors.error} strokeWidth={1.8} />
              <Text style={[styles.overlayText, { color: themeColors.error }]}>
                {tab.error}
              </Text>
              <Pressable
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  reconnectAttemptsRef.current = 0;
                  clearReconnectTimer();
                  connectToTerminal(false);
                }}
                style={[styles.reconnectBtn, { borderColor: themeColors.border }]}
              >
                <RefreshCw size={14} color={themeColors.foreground} strokeWidth={1.8} />
                <Text style={[styles.reconnectText, { color: themeColors.foreground }]}>
                  Reconnect
                </Text>
              </Pressable>
            </>
          ) : (
            <>
              <ActivityIndicator color={themeColors.userMessage} size="small" />
              <Text style={[styles.overlayText, { color: themeColors.mutedForeground }]}>
                {tab.error ?? "Connecting…"}
              </Text>
            </>
          )}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    height: 320,
    borderTopWidth: StyleSheet.hairlineWidth,
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
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexShrink: 1,
  },
  tabLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 999,
  },
  iconBtn: {
    padding: 6,
    borderRadius: 8,
  },
  terminalArea: {
    flex: 1,
    position: "relative",
  },
  terminalPane: {
    ...StyleSheet.absoluteFillObject,
  },
  hiddenPane: {
    display: "none",
  },
  terminalSurface: {
    flex: 1,
    backgroundColor: "#0a0a0a",
  },
  overlay: {
    position: "absolute",
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    alignItems: "center",
    justifyContent: "center",
    gap: 12,
    backgroundColor: "rgba(10,10,10,0.72)",
    paddingHorizontal: 24,
  },
  overlayText: {
    fontSize: 13,
    textAlign: "center",
  },
  reconnectBtn: {
    minHeight: 36,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  reconnectText: {
    fontSize: 13,
    fontWeight: "600",
  },
  centerState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 24,
  },
  stateText: {
    fontSize: 13,
    textAlign: "center",
  },
});
