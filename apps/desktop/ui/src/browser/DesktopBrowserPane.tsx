import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import {
  Bot,
  ExternalLink,
  Globe2,
  Plus,
  RefreshCw,
  Square,
  X,
} from 'lucide-react-native';

import type { BrowserSession, PortEntry } from '@mitsuro/api';
import { useConnection } from '@mobile/hooks/useConnection';
import { useThemeContext } from '@mobile/hooks/useTheme';

type StageMode = 'session' | 'preview';

type AtlasFrame = {
  type: 'frame';
  seq: number;
  data: string;
  metadata?: { deviceWidth?: number; deviceHeight?: number };
};

function AtlasStreamSurface({
  serverUrl,
  serverToken,
  streamPath,
}: {
  serverUrl: string;
  serverToken: string | null;
  streamPath: string;
}) {
  const imageRef = useRef<HTMLImageElement | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const frameRef = useRef<AtlasFrame | null>(null);
  const [status, setStatus] = useState<'connecting' | 'live' | 'error'>('connecting');

  useEffect(() => {
    if (Platform.OS !== 'web') return;
    const base = serverUrl.replace(/^http/i, 'ws').replace(/\/+$/, '');
    const separator = streamPath.includes('?') ? '&' : '?';
    const token = serverToken?.trim()
      ? `&token=${encodeURIComponent(serverToken.trim())}`
      : '';
    const socket = new WebSocket(
      `${base}${streamPath}${separator}capability=controller${token}`,
    );
    socketRef.current = socket;
    socket.onopen = () => {
      setStatus('live');
      socket.send(JSON.stringify({ type: 'config', pacing: 'ack', maxFps: 30 }));
    };
    socket.onerror = () => setStatus('error');
    socket.onclose = () => setStatus((current) => (current === 'error' ? current : 'connecting'));
    socket.onmessage = (event) => {
      if (typeof event.data !== 'string') return;
      try {
        const message = JSON.parse(event.data) as AtlasFrame;
        if (message.type !== 'frame' || !message.data) return;
        frameRef.current = message;
        if (imageRef.current) imageRef.current.src = `data:image/jpeg;base64,${message.data}`;
      } catch {
        // Status, tabs, URL, and console messages are intentionally ignored here.
      }
    };
    return () => {
      socketRef.current = null;
      socket.close();
    };
  }, [serverToken, serverUrl, streamPath]);

  const send = useCallback((payload: Record<string, unknown>) => {
    if (socketRef.current?.readyState === WebSocket.OPEN) {
      socketRef.current.send(JSON.stringify(payload));
    }
  }, []);

  const mousePosition = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const metadata = frameRef.current?.metadata;
    const width = metadata?.deviceWidth ?? bounds.width;
    const height = metadata?.deviceHeight ?? bounds.height;
    return {
      x: ((event.clientX - bounds.left) / bounds.width) * width,
      y: ((event.clientY - bounds.top) / bounds.height) * height,
    };
  }, []);

  if (Platform.OS !== 'web') {
    return (
      <View style={styles.empty}>
        <Text style={styles.streamStatus}>Atlas live stream is available in the desktop/web surface.</Text>
      </View>
    );
  }

  return (
    // This div is the remote Chromium input surface; it never receives raw CDP access.
    <div
      role="application"
      aria-label="Atlas controlled browser"
      tabIndex={0}
      onPointerMove={(event) => {
        const point = mousePosition(event);
        send({ type: 'input_mouse', eventType: 'mouseMoved', ...point, button: 'none' });
      }}
      onPointerDown={(event) => {
        event.currentTarget.focus();
        const point = mousePosition(event);
        const button = event.button === 2 ? 'right' : event.button === 1 ? 'middle' : 'left';
        send({ type: 'input_mouse', eventType: 'mousePressed', ...point, button, clickCount: 1 });
      }}
      onPointerUp={(event) => {
        const point = mousePosition(event);
        const button = event.button === 2 ? 'right' : event.button === 1 ? 'middle' : 'left';
        send({ type: 'input_mouse', eventType: 'mouseReleased', ...point, button, clickCount: 1 });
      }}
      onWheel={(event) => {
        const bounds = event.currentTarget.getBoundingClientRect();
        send({
          type: 'input_mouse',
          eventType: 'mouseWheel',
          x: event.clientX - bounds.left,
          y: event.clientY - bounds.top,
          deltaX: event.deltaX,
          deltaY: event.deltaY,
        });
      }}
      onKeyDown={(event) => {
        event.preventDefault();
        send({
          type: 'input_keyboard',
          eventType: 'keyDown',
          key: event.key,
          code: event.code,
          text: event.key.length === 1 ? event.key : undefined,
          modifiers:
            Number(event.altKey) | (Number(event.ctrlKey) << 1) | (Number(event.metaKey) << 2) | (Number(event.shiftKey) << 3),
        });
      }}
      onKeyUp={(event) => {
        event.preventDefault();
        send({ type: 'input_keyboard', eventType: 'keyUp', key: event.key, code: event.code });
      }}
      onContextMenu={(event) => event.preventDefault()}
      style={{
        position: 'relative',
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        outline: 'none',
        background: '#0b1119',
        touchAction: 'none',
      }}
    >
      <img
        ref={imageRef}
        alt="Atlas browser viewport"
        draggable={false}
        onLoad={() => {
          const frame = frameRef.current;
          if (frame) send({ type: 'ack', seq: frame.seq });
        }}
        style={{ width: '100%', height: '100%', objectFit: 'contain', pointerEvents: 'none', userSelect: 'none' }}
      />
      {status !== 'live' ? (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', color: '#9aa7b5', fontSize: 12 }}>
          {status === 'error' ? 'Atlas stream unavailable' : 'Connecting to Atlas…'}
        </div>
      ) : null}
    </div>
  );
}

function shortId(id: string) {
  return id.slice(0, 8);
}

function statusColor(status: BrowserSession['status'], accent: string, muted: string, danger: string) {
  switch (status) {
    case 'ready':
    case 'running':
      return accent;
    case 'error':
      return danger;
    default:
      return muted;
  }
}

export function DesktopBrowserPane({ visible }: { visible: boolean }) {
  const { theme } = useThemeContext();
  const { client, serverUrl, serverToken } = useConnection();
  const t = theme.colors;

  const [sessions, setSessions] = useState<BrowserSession[]>([]);
  const [ports, setPorts] = useState<PortEntry[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [stageMode, setStageMode] = useState<StageMode>('session');
  const [previewUrl, setPreviewUrl] = useState('');
  const [address, setAddress] = useState('https://example.com');
  const [agentTask, setAgentTask] = useState('Open the page and summarize the main heading.');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [agentBusy, setAgentBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [agentResult, setAgentResult] = useState<string | null>(null);

  const selected = useMemo(
    () => sessions.find((session) => session.id === selectedId) ?? sessions[0] ?? null,
    [selectedId, sessions],
  );

  const refresh = useCallback(async () => {
    if (!client) return;
    setLoading(true);
    try {
      const [browser, portList] = await Promise.all([
        client.listBrowserSessions(),
        client.getPorts().catch(() => ({ ports: [] as PortEntry[] })),
      ]);
      setSessions(browser.sessions);
      setPorts(portList.ports.filter((port) => port.active && port.is_previewable_http));
      setError(null);
      if (!selectedId && browser.sessions[0]) {
        setSelectedId(browser.sessions[0].id);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load browser sessions');
    } finally {
      setLoading(false);
    }
  }, [client, selectedId]);

  useEffect(() => {
    if (!visible) return;
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, 4000);
    return () => clearInterval(timer);
  }, [refresh, visible]);

  useEffect(() => {
    if (!selected || !client) return;
    void client.heartbeatBrowserSession(selected.id, 'viewer').catch(() => undefined);
  }, [client, selected?.id]);

  const createSession = useCallback(
    async (kind: 'interactive' | 'agent') => {
      if (!client) return;
      setBusy(true);
      setError(null);
      try {
        const session = await client.createBrowserSession({
          title: kind === 'agent' ? 'Agent browser' : 'Browser session',
          kind,
          url: address.trim() || undefined,
          launch_local: true,
        });
        setSessions((current) => [session, ...current.filter((item) => item.id !== session.id)]);
        setSelectedId(session.id);
        setStageMode('session');
        setAgentResult(null);
        if (session.last_error) setError(session.last_error);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to create browser session');
      } finally {
        setBusy(false);
      }
    },
    [address, client],
  );

  const stopSession = useCallback(async () => {
    if (!client || !selected) return;
    setBusy(true);
    try {
      const session = await client.stopBrowserSession(selected.id);
      setSessions((current) => current.map((item) => (item.id === session.id ? session : item)));
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to stop session');
    } finally {
      setBusy(false);
    }
  }, [client, selected]);

  const runAgent = useCallback(async () => {
    if (!client || !selected) return;
    const task = agentTask.trim();
    if (!task) return;
    setAgentBusy(true);
    setAgentResult(null);
    setError(null);
    try {
      // Ensure controller presence before automation.
      await client.heartbeatBrowserSession(selected.id, 'controller');
      const result = await client.runBrowserAgent(selected.id, { task, max_steps: 20 });
      if (result.ok) {
        setAgentResult(result.result ?? 'Agent finished.');
      } else {
        setError(result.error ?? 'browser-use agent failed');
        setAgentResult(result.result ?? null);
      }
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'browser-use agent failed');
    } finally {
      setAgentBusy(false);
    }
  }, [agentTask, client, refresh, selected]);

  const openPreviewPort = useCallback(
    (port: number) => {
      if (!serverUrl) return;
      const base = serverUrl.replace(/\/+$/, '');
      setPreviewUrl(`${base}/api/ports/${port}/proxy`);
      setStageMode('preview');
    },
    [serverUrl],
  );

  if (!visible) return null;

  const stageUrl = stageMode === 'preview' ? previewUrl : null;

  return (
    <View style={[styles.root, { backgroundColor: t.background, borderColor: t.border }]}>
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <Globe2 size={14} color={t.mutedForeground} />
        <Text style={[styles.headerTitle, { color: t.mutedForeground }]}>Browser</Text>
        <View style={{ flex: 1 }} />
        <Pressable onPress={() => void refresh()} style={styles.iconBtn} disabled={loading}>
          {loading ? <ActivityIndicator size="small" color={t.mutedForeground} /> : <RefreshCw size={14} color={t.mutedForeground} />}
        </Pressable>
      </View>

      <View style={styles.body}>
        <View style={[styles.side, { borderRightColor: t.border }]}>
          <View style={styles.sideActions}>
            <Pressable
              onPress={() => void createSession('interactive')}
              disabled={busy || !client}
              style={[styles.actionBtn, { borderColor: t.border, backgroundColor: t.glass.background }]}
            >
              <Plus size={13} color={t.foreground} />
              <Text style={[styles.actionText, { color: t.foreground }]}>Session</Text>
            </Pressable>
            <Pressable
              onPress={() => void createSession('agent')}
              disabled={busy || !client}
              style={[styles.actionBtn, { borderColor: t.border, backgroundColor: t.glass.background }]}
            >
              <Bot size={13} color={t.foreground} />
              <Text style={[styles.actionText, { color: t.foreground }]}>Agent</Text>
            </Pressable>
          </View>

          <ScrollView style={styles.sessionList} contentContainerStyle={{ gap: 6, paddingBottom: 12 }}>
            {sessions.length === 0 ? (
              <Text style={{ color: t.mutedForeground, fontSize: 12, paddingHorizontal: 4 }}>
                No sessions
              </Text>
            ) : (
              sessions.map((session) => {
                const active = session.id === selected?.id;
                return (
                  <Pressable
                    key={session.id}
                    onPress={() => {
                      setSelectedId(session.id);
                      setStageMode('session');
                    }}
                    style={[
                      styles.sessionRow,
                      {
                        borderColor: active ? `${t.userMessage}66` : t.border,
                        backgroundColor: active ? `${t.userMessage}12` : t.glass.background,
                      },
                    ]}
                  >
                    <Text style={{ color: t.foreground, fontSize: 12, fontWeight: '600' }} numberOfLines={1}>
                      {session.title}
                    </Text>
                    <Text style={{ color: t.mutedForeground, fontSize: 11 }} numberOfLines={1}>
                      {session.kind} · {session.status} · {shortId(session.id)}
                    </Text>
                  </Pressable>
                );
              })
            )}
          </ScrollView>

          {ports.length > 0 ? (
            <View style={[styles.portBlock, { borderTopColor: t.border }]}>
              <Text style={[styles.metaLabel, { color: t.mutedForeground }]}>Previews</Text>
              <View style={styles.portChips}>
                {ports.slice(0, 6).map((port) => (
                  <Pressable
                    key={`${port.port}-${port.name}`}
                    onPress={() => openPreviewPort(port.port)}
                    style={[styles.portChip, { borderColor: t.border }]}
                  >
                    <Text style={{ color: t.foreground, fontSize: 11 }}>:{port.port}</Text>
                  </Pressable>
                ))}
              </View>
            </View>
          ) : null}
        </View>

        <View style={styles.main}>
          <View style={[styles.toolbar, { borderBottomColor: t.border }]}>
            <TextInput
              value={address}
              onChangeText={setAddress}
              placeholder="Start URL"
              placeholderTextColor={t.mutedForeground}
              autoCapitalize="none"
              autoCorrect={false}
              style={[
                styles.address,
                {
                  color: t.foreground,
                  borderColor: t.border,
                  backgroundColor: t.glass.background,
                },
              ]}
            />
            {selected ? (
              <Pressable onPress={() => void stopSession()} style={styles.iconBtn} disabled={busy}>
                <Square size={13} color={t.mutedForeground} />
              </Pressable>
            ) : null}
            {stageUrl ? (
              <Pressable
                onPress={() => {
                  if (typeof window !== 'undefined') window.open(stageUrl, '_blank', 'noopener,noreferrer');
                }}
                style={styles.iconBtn}
              >
                <ExternalLink size={13} color={t.mutedForeground} />
              </Pressable>
            ) : null}
          </View>

          {selected ? (
            <View style={[styles.metaRow, { borderBottomColor: t.border }]}>
              <Text style={{ color: statusColor(selected.status, t.userMessage, t.mutedForeground, t.destructive ?? '#ef4444'), fontSize: 12 }}>
                {selected.status}
              </Text>
              <Text style={{ color: t.mutedForeground, fontSize: 12 }}>
                {selected.cdp_url ? `cdp ${selected.debug_port ?? ''}` : 'no cdp'}
              </Text>
              <Text style={{ color: t.mutedForeground, fontSize: 12 }}>
                v{selected.viewers}/c{selected.controllers}
              </Text>
              {selected.last_error ? (
                <Text style={{ color: t.destructive ?? '#ef4444', fontSize: 12 }} numberOfLines={1}>
                  {selected.last_error}
                </Text>
              ) : null}
            </View>
          ) : null}

          <View style={styles.stage}>
            {stageMode === 'session' && selected?.stream_url && serverUrl ? (
              <AtlasStreamSurface
                serverUrl={serverUrl}
                serverToken={serverToken}
                streamPath={selected.stream_url}
              />
            ) : stageUrl && Platform.OS === 'web' ? (
              <iframe
                src={stageUrl}
                title={selected?.title ?? 'Browser'}
                style={{ border: '0', width: '100%', height: '100%', background: '#0b1119' }}
                sandbox="allow-downloads allow-forms allow-modals allow-pointer-lock allow-popups allow-presentation allow-scripts allow-same-origin"
              />
            ) : (
              <View style={styles.empty}>
                <Globe2 size={22} color={t.mutedForeground} />
                <Text style={{ color: t.mutedForeground, fontSize: 12, textAlign: 'center', maxWidth: 320, lineHeight: 17 }}>
                  {selected
                    ? 'Atlas is running, but its live stream is unavailable.'
                    : 'Create a session or open a local preview'}
                </Text>
              </View>
            )}
          </View>

          {selected ? (
          <View style={[styles.agentBar, { borderTopColor: t.border }]}>
            <TextInput
              value={agentTask}
              onChangeText={setAgentTask}
              placeholder="Agent task"
              placeholderTextColor={t.mutedForeground}
              autoCapitalize="none"
              autoCorrect={false}
              style={[
                styles.agentInput,
                {
                  color: t.foreground,
                  borderColor: t.border,
                  backgroundColor: t.glass.background,
                },
              ]}
            />
            <Pressable
              onPress={() => void runAgent()}
              disabled={!selected || agentBusy || !client}
              style={[
                styles.runBtn,
                {
                  borderColor: t.border,
                  backgroundColor: selected ? `${t.userMessage}22` : t.glass.background,
                  opacity: !selected || agentBusy ? 0.55 : 1,
                },
              ]}
            >
              {agentBusy ? (
                <ActivityIndicator size="small" color={t.userMessage} />
              ) : (
                <Bot size={14} color={t.userMessage} />
              )}
              <Text style={{ color: t.foreground, fontSize: 12, fontWeight: '600' }}>Run</Text>
            </Pressable>
            {agentResult ? (
              <Pressable onPress={() => setAgentResult(null)} style={styles.iconBtn}>
                <X size={13} color={t.mutedForeground} />
              </Pressable>
            ) : null}
          </View>
          ) : null}

          {error ? (
            <Text style={[styles.footerNote, { color: t.destructive ?? '#ef4444' }]} numberOfLines={3}>
              {error}
            </Text>
          ) : null}
          {agentResult ? (
            <ScrollView style={styles.resultBox} contentContainerStyle={{ padding: 8 }}>
              <Text style={{ color: t.mutedForeground, fontSize: 12, lineHeight: 17 }}>{agentResult}</Text>
            </ScrollView>
          ) : null}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  header: {
    minHeight: 36,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  headerTitle: {
    fontSize: 12,
    fontWeight: '700',
    letterSpacing: 0.2,
  },
  body: {
    flex: 1,
    flexDirection: 'row',
  },
  side: {
    width: 220,
    borderRightWidth: StyleSheet.hairlineWidth,
    padding: 8,
    gap: 8,
  },
  sideActions: {
    flexDirection: 'row',
    gap: 6,
  },
  actionBtn: {
    flex: 1,
    minHeight: 30,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    alignItems: 'center',
    justifyContent: 'center',
    flexDirection: 'row',
    gap: 5,
  },
  actionText: {
    fontSize: 12,
    fontWeight: '600',
  },
  sessionList: {
    flex: 1,
  },
  sessionRow: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 8,
    paddingVertical: 7,
    gap: 2,
  },
  portBlock: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 8,
    gap: 6,
  },
  metaLabel: {
    fontSize: 10,
    fontWeight: '700',
    textTransform: 'uppercase',
    letterSpacing: 0.4,
  },
  portChips: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 6,
  },
  portChip: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 3,
  },
  main: {
    flex: 1,
  },
  toolbar: {
    minHeight: 40,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 8,
    gap: 6,
    flexDirection: 'row',
    alignItems: 'center',
  },
  address: {
    flex: 1,
    height: 30,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 10,
    fontSize: 12,
  },
  iconBtn: {
    width: 28,
    height: 28,
    borderRadius: 8,
    alignItems: 'center',
    justifyContent: 'center',
  },
  metaRow: {
    minHeight: 28,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    gap: 10,
    flexDirection: 'row',
    alignItems: 'center',
  },
  stage: {
    flex: 1,
    position: 'relative',
  },
  empty: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 24,
    gap: 8,
  },
  streamStatus: {
    color: '#9aa7b5',
    fontSize: 12,
    textAlign: 'center',
  },
  agentBar: {
    minHeight: 44,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 8,
    paddingVertical: 6,
    gap: 6,
    flexDirection: 'row',
    alignItems: 'center',
  },
  agentInput: {
    flex: 1,
    height: 32,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 10,
    fontSize: 12,
  },
  runBtn: {
    minHeight: 32,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 10,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  footerNote: {
    paddingHorizontal: 10,
    paddingVertical: 6,
    fontSize: 12,
  },
  resultBox: {
    maxHeight: 120,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
});
