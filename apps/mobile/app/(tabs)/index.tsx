import { useState, useRef, useCallback, useEffect } from 'react';
import {
  View,
  FlatList,
  StyleSheet,
  Text,
  Pressable,
  Alert,
  Keyboard,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { router } from 'expo-router';
import { Menu, FileSearch } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import * as SecureStore from '../../platform/secure-store';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import { MessageBubble } from '../../components/chat/MessageBubble';
import { KrustyLogo } from '../../components/ui/KrustyLogo';
import { ChatBar } from '../../components/chat/ChatBar';
import { SessionDrawer } from '../../components/chat/SessionDrawer';
import { DesktopShell } from '../../components/layout/DesktopShell';
import { ReportsViewer } from '../../components/ReportsViewer';
import { LinearGradient } from '../../platform/linear-gradient';
import type { ChatMessage, ModelInfo, SessionResponse, SessionType, ThinkingLevel } from '@krusty/api';

const TAB_TYPES: SessionType[] = ['chat', 'code', 'mako'];

function sessionTypeForTab(index: number): SessionType {
  return TAB_TYPES[index] ?? 'code';
}

function tabForSessionType(type: SessionType): number {
  switch (type) {
    case 'chat':
      return 0;
    case 'mako':
      return 2;
    default:
      return 1;
  }
}

export default function ChatScreen() {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();

  // Session state
  const [sessions, setSessions] = useState<SessionResponse[]>([]);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessionTitle, setSessionTitle] = useState('');
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  // Chat state
  const [isStreaming, setIsStreaming] = useState(false);
  const [isThinking, setIsThinking] = useState(false);
  const [model, setModel] = useState<string | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>('off');
  const [mode, setMode] = useState<'build' | 'plan'>('build');
  const [researchEnabled, setResearchEnabled] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [pendingApproval, setPendingApproval] = useState<{id: string; name: string; args: Record<string, unknown>} | null>(null);

  // UI state
  const [activeTab, setActiveTab] = useState(1); // 0=Chat, 1=Code, 2=Mako
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reportsOpen, setReportsOpen] = useState(false);
  const flatListRef = useRef<FlatList>(null);
  const abortRef = useRef<AbortController | null>(null);

  // Mutable ref for the current assistant message being streamed.
  // Avoids recreating the entire messages array on every delta.
  const assistantRef = useRef<ChatMessage>({ role: 'assistant', content: '', toolCalls: [] });
  const flushTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const t = theme.colors;

  // Load sessions + models on connect, restore persisted model
  useEffect(() => {
    if (client && isConnected) {
      client.getSessions().then(setSessions).catch(() => {});
      client.getModels().then(async (res) => {
        setModels(res.models);
        if (!model) {
          const saved = await SecureStore.getItemAsync('krusty_selected_model');
          const validSaved = saved && res.models.some((m: { id: string }) => m.id === saved);
          setModel(validSaved ? saved : res.default_model);
        }
      }).catch(() => {});
    }
  }, [client, isConnected]);

  // Load session messages
  const loadSession = useCallback(async (session: SessionResponse) => {
    if (!client) return;
    setSessionId(session.id);
    setSessionTitle(session.title || 'Untitled');
    setModel(session.model ?? null);
    setMode(session.mode ?? 'build');
    setTokenCount(session.token_count ?? 0);
    setActiveTab(tabForSessionType(session.session_type));
    setDrawerOpen(false);

    try {
      const data = await client.getSession(session.id);
      const loaded: ChatMessage[] = [];

      for (const msg of data.messages) {
        const textParts = msg.content.filter(c => c.type === 'text').map(c => c.text ?? '').join('');
        const thinkingParts = msg.content.filter(c => c.type === 'thinking').map(c => c.thinking ?? '').join('');
        const toolUses = msg.content.filter(c => c.type === 'tool_use');
        const toolResults = new Map(
          msg.content.filter(c => c.type === 'tool_result').map(c => [c.tool_use_id, c.content ?? ''])
        );

        loaded.push({
          role: msg.role,
          content: textParts,
          thinking: thinkingParts || undefined,
          toolCalls: toolUses.map(tu => ({
            id: tu.id!,
            name: tu.name!,
            arguments: tu.input,
            output: toolResults.get(tu.id!) ?? undefined,
            status: toolResults.has(tu.id!) ? 'success' as const : 'pending' as const,
          })),
        });
      }

      setMessages(loaded);
    } catch {
      setMessages([]);
    }
  }, [client]);

  const handleNewSession = useCallback(async () => {
    if (!client) return;
    // Chat sessions start immediately with no directory.
    // Code/Mako use the inline directory picker in the SessionDrawer.
    try {
      const session = await client.createSession(undefined, undefined, undefined, undefined, 'chat');
      setSessions(prev => [session, ...prev]);
      setSessionId(session.id);
      setSessionTitle(session.title || '');
      setMessages([]);
      setDrawerOpen(false);
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch {
      // silent
    }
  }, [client]);

  const handleDirectorySelected = useCallback(async (path: string) => {
    if (!client) return;
    try {
      const type = sessionTypeForTab(activeTab);
      const session = await client.createSession(undefined, path, undefined, 'selected', type);
      setSessions(prev => [session, ...prev]);
      setSessionId(session.id);
      setSessionTitle(session.title || '');
      setMessages([]);
      setDrawerOpen(false);
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch {
      // silent
    }
  }, [activeTab, client]);

  const handleDeleteSession = useCallback(async (id: string) => {
    if (!client) return;
    Alert.alert('Delete Session', 'Delete this session?', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          try {
            await client.deleteSession(id);
            setSessions(prev => prev.filter(s => s.id !== id));
            if (sessionId === id) {
              setSessionId(null);
              setSessionTitle('');
              setMessages([]);
            }
          } catch { /* silent */ }
        },
      },
    ]);
  }, [client, sessionId]);

  // Flush the mutable assistant ref into React state.
  // Called on a 50ms interval during streaming (throttled re-renders)
  // and once on stream end for the final state.
  const flushAssistantRef = useCallback(() => {
    const snapshot = { ...assistantRef.current, toolCalls: [...(assistantRef.current.toolCalls ?? [])] };
    setMessages(prev => {
      const updated = [...prev];
      if (updated.length > 0 && updated[updated.length - 1].role === 'assistant') {
        updated[updated.length - 1] = snapshot;
      }
      return updated;
    });
  }, []);

  // Auto-scroll: runs after each render when messages change during streaming
  const msgLen = messages.length;
  const lastMsgContent = messages[msgLen - 1]?.content?.length ?? 0;
  useEffect(() => {
    if (msgLen > 0) {
      requestAnimationFrame(() => {
        flatListRef.current?.scrollToEnd({ animated: !isStreaming });
      });
    }
  }, [msgLen, lastMsgContent, isStreaming]);

  const startFlushTimer = useCallback(() => {
    if (flushTimerRef.current) return;
    flushTimerRef.current = setInterval(flushAssistantRef, 50);
  }, [flushAssistantRef]);

  const stopFlushTimer = useCallback(() => {
    if (flushTimerRef.current) {
      clearInterval(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    flushAssistantRef(); // final flush
  }, [flushAssistantRef]);

  const handleSend = useCallback(async (content: string) => {
    if (!client || !content.trim()) return;

    // Reset assistant ref for new message
    assistantRef.current = { role: 'assistant', content: '', toolCalls: [] };

    const userMessage: ChatMessage = { role: 'user', content: content.trim(), toolCalls: [] };
    setMessages(prev => [...prev, userMessage, assistantRef.current]);
    setIsStreaming(true);
    setIsThinking(false);

    const abort = new AbortController();
    abortRef.current = abort;
    startFlushTimer();

    try {
      let currentSessionId = sessionId;

      if (!currentSessionId) {
        const session = await client.createSession(
          undefined,
          undefined,
          undefined,
          undefined,
          sessionTypeForTab(activeTab),
        );
        currentSessionId = session.id;
        setSessionId(session.id);
        setSessionTitle('');
        setSessions(prev => [session, ...prev]);
      }

      await client.streamChat(
        {
          session_id: currentSessionId!,
          message: content.trim(),
          model: model ?? undefined,
          thinking_enabled: thinkingLevel !== 'off' ? thinkingLevel : undefined,
          mode,
          research_enabled: researchEnabled || undefined,
        },
        {
          onTextDelta: (delta: string) => {
            assistantRef.current.content += delta;
            setIsThinking(false);
          },
          onThinkingDelta: (thinking: string) => {
            assistantRef.current.thinking = (assistantRef.current.thinking ?? '') + thinking;
            setIsThinking(true);
          },
          onToolCallStart: (id: string, name: string) => {
            setIsThinking(false);
            const toolCalls = assistantRef.current.toolCalls ?? [];
            toolCalls.push({ id, name, status: 'running' as const });
            assistantRef.current.toolCalls = toolCalls;
            flushAssistantRef(); // immediate flush for tool visibility
          },
          onToolCallComplete: (id: string, _name: string, args: Record<string, unknown>) => {
            const toolCalls = assistantRef.current.toolCalls ?? [];
            const tc = toolCalls.find(t => t.id === id);
            if (tc) tc.arguments = args;
          },
          onToolResult: (id: string, output: string, isError: boolean) => {
            const toolCalls = assistantRef.current.toolCalls ?? [];
            const tc = toolCalls.find(t => t.id === id);
            if (tc) {
              tc.output = output;
              tc.status = isError ? 'error' as const : 'success' as const;
            }
            flushAssistantRef(); // immediate flush for result visibility
          },
          onToolOutputDelta: (id: string, delta: string) => {
            const toolCalls = assistantRef.current.toolCalls ?? [];
            const tc = toolCalls.find(t => t.id === id);
            if (tc) tc.output = (tc.output ?? '') + delta;
          },
          onDelegatedProgress: () => {},
          onToolApprovalRequired: (id: string, name: string, _args: Record<string, unknown>) => {
            setPendingApproval({ id, name, args: _args });
            Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
            Alert.alert(
              'Tool Approval',
              `Allow "${name}" to execute?`,
              [
                { text: 'Deny', style: 'destructive', onPress: () => {
                  client.submitToolApproval(currentSessionId!, id, false).catch(() => {});
                  setPendingApproval(null);
                }},
                { text: 'Allow', onPress: () => {
                  client.submitToolApproval(currentSessionId!, id, true).catch(() => {});
                  setPendingApproval(null);
                }},
              ],
            );
          },
          onToolApproved: () => setPendingApproval(null),
          onToolDenied: () => setPendingApproval(null),
          onTurnComplete: (_turn: number, hasMore: boolean) => {
            if (hasMore) {
              // New turn — snapshot current message and start fresh
              flushAssistantRef();
              assistantRef.current = { role: 'assistant', content: '', toolCalls: [] };
              setMessages(prev => [...prev, assistantRef.current]);
            }
          },
          onPlanUpdate: () => {},
          onModeChange: (newMode: string) => {
            if (newMode === 'build' || newMode === 'plan') setMode(newMode);
          },
          onPlanComplete: () => {},
          onUsage: (prompt: number, completion: number) => { setTokenCount(prompt + completion); },
          onTitleUpdate: (title: string) => {
            setSessionTitle(title);
            setSessions(prev => prev.map(s => s.id === currentSessionId ? { ...s, title } : s));
          },
          onFinish: () => { stopFlushTimer(); setIsStreaming(false); setIsThinking(false); },
          onError: () => { stopFlushTimer(); setIsStreaming(false); setIsThinking(false); },
        },
        abort.signal,
      );
    } catch {
      stopFlushTimer();
      setIsStreaming(false);
      setIsThinking(false);
    }
  }, [activeTab, client, sessionId, model, thinkingLevel, mode, startFlushTimer, stopFlushTimer, flushAssistantRef]);

  const handleStop = useCallback(() => {
    abortRef.current?.abort();
    setIsStreaming(false);
  }, []);

  const handleModelSelect = (modelId: string) => {
    setModel(modelId);
    SecureStore.setItemAsync('krusty_selected_model', modelId).catch(() => {});
  };

  const handleTabChange = useCallback((index: number) => {
    setActiveTab(index);
    const currentSession = sessions.find(session => session.id === sessionId);
    if (currentSession && currentSession.session_type !== sessionTypeForTab(index)) {
      setSessionId(null);
      setSessionTitle('');
      setMessages([]);
      setMode('build');
    }
  }, [sessionId, sessions]);


  const chatContent = (
    <SafeAreaView style={[styles.container, { backgroundColor: t.background }]} edges={isDesktop ? [] : ['top']}>
      {/* Top bar */}
      <View style={styles.topBar}>
        {!isDesktop && (
          <Pressable
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setDrawerOpen(true);
            }}
            style={styles.menuBtn}
          >
            <Menu size={22} color={t.foreground} strokeWidth={1.8} />
          </Pressable>
        )}

        <Pressable
          onPress={() => {
            if (!sessionId || !client || !sessionTitle) return;
            Alert.prompt(
              'Rename Session',
              undefined,
              async (newTitle: string) => {
                if (newTitle && newTitle.trim()) {
                  setSessionTitle(newTitle.trim());
                  await client.updateSession(sessionId, { title: newTitle.trim() });
                  setSessions(prev => prev.map(s => s.id === sessionId ? { ...s, title: newTitle.trim() } : s));
                }
              },
              'plain-text',
              sessionTitle,
            );
          }}
          style={styles.titleBtn}
          disabled={!sessionTitle}
        >
          <Text style={[styles.title, { color: sessionTitle ? t.foreground : 'transparent' }]} numberOfLines={1}>
            {sessionTitle || ' '}
          </Text>
        </Pressable>

        <Pressable
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setReportsOpen(true);
          }}
          style={styles.menuBtn}
        >
          <FileSearch size={20} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      </View>

      {/* Messages */}
      {messages.length === 0 ? (
        <Pressable style={styles.empty} onPress={Keyboard.dismiss}>
          <KrustyLogo />
        </Pressable>
      ) : (
        <View style={styles.messagesWrap}>
          <FlatList
            ref={flatListRef}
            data={messages}
            keyExtractor={(_, i) => String(i)}
            onScrollBeginDrag={Keyboard.dismiss}
            renderItem={({ item, index }: { item: ChatMessage; index: number }) => (
              <MessageBubble
                message={item}
                isLast={index === messages.length - 1}
                isStreaming={isStreaming && index === messages.length - 1}
                isThinking={isThinking && index === messages.length - 1}
              />
            )}
            style={styles.flex}
            contentContainerStyle={[styles.list, isDesktop && styles.listDesktop]}
            keyboardDismissMode="interactive"
            keyboardShouldPersistTaps="handled"
          />
          {/* Fade edges */}
          <LinearGradient
            colors={[t.background, t.background + '00']}
            style={styles.fadeTop}
            pointerEvents="none"
          />
          <LinearGradient
            colors={[t.background + '00', t.background]}
            style={styles.fadeBottom}
            pointerEvents="none"
          />
        </View>
      )}

      {/* Chat bar */}
      <ChatBar
        onSend={handleSend}
        onStop={handleStop}
        isStreaming={isStreaming}
        disabled={!isConnected}
        thinkingLevel={thinkingLevel}
        onThinkingChange={setThinkingLevel}
        mode={mode}
        onModeToggle={() => setMode(m => m === 'build' ? 'plan' : 'build')}
        onModelSelect={handleModelSelect}
        model={model}
        models={models}
        sessionType={sessionTypeForTab(activeTab)}
        researchEnabled={researchEnabled}
        onResearchToggle={() => setResearchEnabled(r => !r)}
        tokenCount={tokenCount}
      />

      {/* Reports viewer */}
      <ReportsViewer visible={reportsOpen} onClose={() => setReportsOpen(false)} />
    </SafeAreaView>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={loadSession}
      onNewSession={handleNewSession}
      onNewSessionWithDir={handleDirectorySelected}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.push('/(tabs)/settings')}
      activeTab={activeTab}
      onTabChange={handleTabChange}
    >
      {chatContent}

      {/* Session drawer — mobile only */}
      {!isDesktop && (
        <SessionDrawer
          isOpen={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={loadSession}
          onNewSession={handleNewSession}
          onNewSessionWithDir={handleDirectorySelected}
          onDeleteSession={handleDeleteSession}
          onOpenSettings={() => { setDrawerOpen(false); router.push('/(tabs)/settings'); }}
          activeTab={activeTab}
          onTabChange={handleTabChange}
        />
      )}
    </DesktopShell>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  messagesWrap: { flex: 1, minHeight: 0 },
  topBar: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  menuBtn: {
    padding: 4,
  },
  titleBtn: {
    flex: 1,
  },
  title: {
    fontSize: 17,
    fontWeight: '600',
    textAlign: 'center',
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  list: {
    paddingHorizontal: 16,
    paddingTop: 8,
    paddingBottom: 8,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: 'center',
    width: '100%',
  },
  fadeTop: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    height: 64,
  },
  fadeBottom: {
    position: 'absolute',
    bottom: 0,
    left: 0,
    right: 0,
    height: 120,
  },
  empty: {
    flex: 1,
    justifyContent: 'flex-start',
    alignItems: 'center',
    paddingTop: '35%',
    gap: 16,
  },
  emptyTitle: {
    fontSize: 28,
    fontWeight: '700',
    letterSpacing: -0.5,
  },
  emptyHint: {
    fontSize: 17,
  },
  stubTitle: {
    fontSize: 24,
    fontWeight: '700',
    letterSpacing: -0.3,
  },
  stubText: {
    fontSize: 15,
    marginTop: 8,
  },
  modalBackdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.6)',
    justifyContent: 'flex-end',
    zIndex: 200,
  },
  modelPicker: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: '60%',
    paddingTop: 20,
    paddingBottom: 40,
    backgroundColor: '#1a1f2e',
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: 'rgba(255,255,255,0.1)',
  },
  modelPickerTitle: {
    fontSize: 18,
    fontWeight: '700',
    textAlign: 'center',
    marginBottom: 16,
  },
  modelList: {
    paddingHorizontal: 16,
  },
  modelItem: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 8,
  },
  modelName: {
    fontSize: 16,
    fontWeight: '500',
  },
  modelProvider: {
    fontSize: 13,
    marginTop: 2,
  },
});
