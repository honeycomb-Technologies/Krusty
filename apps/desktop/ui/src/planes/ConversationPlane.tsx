import { useCallback, useEffect, useMemo, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { PanelRightOpen, Wrench } from 'lucide-react-native';
import type { ModelInfo, SessionType } from '@mitsuro/api';
import type { PermissionMode, ThinkingLevel } from '@mitsuro/state';
import { resolveUsableModel, supportsFastMode } from '@mitsuro/state';
import { ChatTranscript } from '@mobile/components/chat/ChatTranscript';
import { ChatBar, type Attachment as ChatBarAttachment } from '@mobile/components/chat/ChatBar';
import { useConnection } from '@mobile/hooks/useConnection';
import {
  useSessionStore,
  useStores,
  useWorkspaceStore,
} from '@mobile/hooks/useStores';
import { useThemeContext } from '@mobile/hooks/useTheme';
import * as SecureStore from '@mobile/platform/secure-store';
import { displayThreadTitle } from '@mobile/components/navigation/threadTitle';

const SELECTED_MODEL_KEY = 'mitsuro:selected-model';

function flattenToolCalls(messages: any[]) {
  const tools: any[] = [];
  for (const message of messages) {
    if (message.toolCalls?.length) tools.push(...message.toolCalls);
  }
  return tools;
}

export function ConversationPlane({
  plane,
  utilityOpen,
  onToggleUtility,
  onOpenSettings,
  onOpenProject,
}: {
  plane: SessionType;
  utilityOpen: boolean;
  onToggleUtility: () => void;
  onOpenSettings: () => void;
  onOpenProject?: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const { client, isConnected } = useConnection();
  const stores = useStores();
  const modeStores = stores?.modes[plane];
  const sessionStore = modeStores?.session;
  const workspaceStore = modeStores?.workspace;
  const sessionsStore = stores?.sessions;

  const sessionId = useSessionStore((state) => state.sessionId, plane) ?? null;
  const sessionTitle = useSessionStore((state) => state.title, plane) ?? null;
  const messages = useSessionStore((state) => state.messages, plane) ?? [];
  const isStreaming = useSessionStore((state) => state.isStreaming, plane) ?? false;
  const isThinking = useSessionStore((state) => state.isThinking, plane) ?? false;
  const model = useSessionStore((state) => state.model, plane) ?? null;
  const thinkingLevel = useSessionStore((state) => state.thinkingLevel, plane) ?? 'medium';
  const permissionMode = useSessionStore((state) => state.permissionMode, plane) ?? 'autonomous';
  const fastModeStoreEnabled = useSessionStore((state) => state.fastModeEnabled, plane) ?? false;
  const mode = useSessionStore((state) => state.mode, plane) ?? 'build';
  const tokenCount = useSessionStore((state) => state.tokenCount, plane) ?? 0;
  const error = useSessionStore((state) => state.error, plane) ?? null;
  const workspaceDirectory = useWorkspaceStore((state) => state.directory, plane) ?? null;
  const workspaceTargetBranch = useWorkspaceStore((state) => state.targetBranch, plane) ?? null;

  const [models, setModels] = useState<ModelInfo[]>([]);
  const [composerReserveHeight, setComposerReserveHeight] = useState(132);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);

  const selectedModelInfo = useMemo(
    () => models.find((candidate) => candidate.id === model) ?? null,
    [model, models],
  );
  const fastModeSupported = supportsFastMode(selectedModelInfo ?? model, selectedModelInfo?.provider ?? null);
  const fastModeEnabled = fastModeSupported && fastModeStoreEnabled;
  const displayTitle = displayThreadTitle(sessionTitle);
  const hasHeaderContent = Boolean(displayTitle || (plane === 'code' && workspaceDirectory) || !isConnected);
  const toolCalls = useMemo(() => flattenToolCalls(messages), [messages]);

  useEffect(() => {
    if (!client || !isConnected || !sessionStore) return;
    let cancelled = false;
    (async () => {
      try {
        const [catalog, credentials] = await Promise.all([
          client.getModels(),
          client.getCredentials().catch(() => []),
        ]);
        if (cancelled) return;
        const nextModels = catalog.models ?? [];
        setModels(nextModels);
        const configuredProviders = credentials
          .filter((provider: any) => provider.configured || provider.has_oauth)
          .map((provider: any) => String(provider.name || '').trim().toLowerCase());
        const saved = await SecureStore.getItemAsync(SELECTED_MODEL_KEY);
        const usable = resolveUsableModel(
          sessionStore.getState().model ?? saved,
          catalog.default_model ?? null,
          nextModels,
          configuredProviders,
        );
        if (usable) {
          sessionStore
            .getState()
            .setModel(usable.id, usable.provider ?? null, usable);
          await SecureStore.setItemAsync(SELECTED_MODEL_KEY, usable.id);
        }
      } catch {
        // ignore catalog failures; composer still works with server defaults
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, isConnected, sessionStore]);

  useEffect(() => {
    if (!toolCalls.length) {
      setActiveToolCallId(null);
      return;
    }
    const awaiting = toolCalls.find((tool) => tool.status === 'awaiting_approval');
    const running = toolCalls.find((tool) => tool.status === 'running' || tool.status === 'pending');
    setActiveToolCallId(awaiting?.id ?? running?.id ?? toolCalls[toolCalls.length - 1]?.id ?? null);
  }, [toolCalls]);

  const handleSend = useCallback(
    async (content: string, attachments?: ChatBarAttachment[]) => {
      if (!sessionStore || !content.trim()) return;
      if (!sessionStore.getState().sessionId && client) {
        const created = await client.createSession(
          undefined,
          plane === 'code' ? workspaceDirectory ?? undefined : undefined,
          plane === 'code' ? workspaceTargetBranch ?? undefined : undefined,
          plane === 'code' && workspaceDirectory ? 'selected' : 'neutral',
          plane,
          sessionStore.getState().permissionMode,
        );
        sessionStore.getState().initSession(
          created.id,
          created.title || '',
          created.permission_mode,
          created.session_type,
        );
        if (workspaceStore) {
          workspaceStore.getState().initFromSession(
            created.id,
            created.project_dir ?? created.working_dir ?? null,
            (created.workspace_mode ?? (created.project_dir || created.working_dir ? 'selected' : 'neutral')) as any,
            created.target_branch ?? null,
          );
        }
        await sessionsStore?.getState().loadSessions();
      }
      const mapped = (attachments ?? []).map((attachment) => ({
        type: attachment.type,
        name: attachment.name ?? 'attachment',
        uri: attachment.uri,
        base64: attachment.base64,
        mimeType: attachment.mimeType ?? 'application/octet-stream',
      }));
      await sessionStore.getState().sendMessage(content, mapped, {
        sessionType: plane,
        projectDir: plane === 'code' ? workspaceDirectory : undefined,
        workingDir: plane === 'code' ? workspaceDirectory : undefined,
        targetBranch: plane === 'code' ? workspaceTargetBranch : undefined,
      });
    },
    [client, plane, sessionStore, sessionsStore, workspaceDirectory, workspaceStore, workspaceTargetBranch],
  );

  const handleStop = useCallback(() => {
    sessionStore?.getState().stopStreaming();
  }, [sessionStore]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Enter' || event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      if ((event as any).isComposing) return;
      const target = event.target as HTMLElement | null;
      if (!target) return;
      const tag = (target.tagName || '').toLowerCase();
      const aria = target.getAttribute('aria-label') || '';
      const isComposer =
        tag === 'textarea' ||
        aria === 'Message Mitsuro...' ||
        aria === 'Message Hive...';
      if (!isComposer) return;
      const value = 'value' in (target as any) ? String((target as HTMLTextAreaElement).value || '') : '';
      const trimmed = value.trim();
      if (!trimmed || isStreaming) return;
      event.preventDefault();
      event.stopPropagation();
      void handleSend(trimmed);
      // Clear the DOM field immediately for snappy desktop feel.
      try {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')?.set;
        setter?.call(target, '');
        target.dispatchEvent(new Event('input', { bubbles: true }));
      } catch {
        // ignore
      }
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [handleSend, isStreaming]);

  const handleModelSelect = useCallback(
    async (modelId: string) => {
      sessionStore?.getState().setModel(modelId);
      await SecureStore.setItemAsync(SELECTED_MODEL_KEY, modelId);
    },
    [sessionStore],
  );

  const handleApprove = useCallback(
    async (_targetSessionId: string, toolCallId: string, approved: boolean) => {
      if (!sessionStore) return;
      await sessionStore.getState().submitToolApproval(toolCallId, approved);
    },
    [sessionStore],
  );

  if (!stores || !sessionStore) {
    return (
      <View style={[styles.center, { backgroundColor: t.background }]}> 
        <Text style={{ color: t.mutedForeground }}>Connecting stores…</Text>
      </View>
    );
  }

  return (
    <View style={[styles.root, { backgroundColor: t.background }]}> 
      <View style={[styles.topBar, !hasHeaderContent && styles.topBarCompact, { borderBottomColor: t.border }]}>
        <View style={styles.titleBlock}>
          {displayTitle || (plane === 'code' && workspaceDirectory) ? (
            <>
              <Text style={[styles.title, { color: t.foreground }]} numberOfLines={1}>
                {displayTitle ||
                  workspaceDirectory?.split('/').filter(Boolean).slice(-1)[0] ||
                  ''}
              </Text>
              {plane === 'code' && workspaceDirectory ? (
                <Text style={[styles.meta, { color: t.mutedForeground }]} numberOfLines={1}>
                  {workspaceDirectory}
                  {workspaceTargetBranch ? ` · ${workspaceTargetBranch}` : ''}
                </Text>
              ) : null}
            </>
          ) : null}
        </View>
        <View style={styles.topActions}>
          {!isConnected ? (
            <Pressable onPress={onOpenSettings} style={[styles.chip, { borderColor: `${t.error}66` }]}>
              <Text style={{ color: t.error, fontSize: 12, fontWeight: '600' }}>Connect</Text>
            </Pressable>
          ) : null}
          <Pressable
            onPress={onToggleUtility}
            accessibilityLabel={utilityOpen ? 'Close tools' : 'Open tools'}
            style={[
              styles.iconChip,
              {
                borderColor: utilityOpen ? `${t.userMessage}66` : t.border,
                backgroundColor: utilityOpen ? `${t.userMessage}14` : 'transparent',
              },
            ]}
          >
            {utilityOpen ? (
              <PanelRightOpen size={15} color={t.userMessage} />
            ) : (
              <Wrench size={15} color={t.mutedForeground} />
            )}
          </Pressable>
        </View>
      </View>

      <View style={styles.canvas}>
        <View style={styles.column}>
          <ChatTranscript
            key={`${plane}:${sessionId ?? 'new'}`}
            messages={messages}
            sessionId={sessionId}
            sessionType={plane}
            scrollStateKey={`${plane}:${sessionId ?? 'new'}`}
            isStreaming={isStreaming}
            isThinking={isThinking}
            activeToolCallId={activeToolCallId}
            onApproveTool={(targetSessionId, toolCallId) => void handleApprove(targetSessionId, toolCallId, true)}
            onDenyTool={(targetSessionId, toolCallId) => void handleApprove(targetSessionId, toolCallId, false)}
            onSubmitToolResult={(toolCallId, result) => void sessionStore.getState().submitToolResult(toolCallId, result)}
            onPlanConfirm={(toolCallId, choice) => void sessionStore.getState().submitToolResult(toolCallId, JSON.stringify({ choice }))}
            emptyState={
              <View style={styles.empty}>
                {error ? <Text style={{ color: t.error }}>{error}</Text> : null}
              </View>
            }
            bottomPadding={composerReserveHeight}
          />

          <View style={styles.composer}>
            <ChatBar
              draftKey={`desktop:${plane}`}
              onSend={(content, attachments) => void handleSend(content, attachments)}
              onStop={handleStop}
              onHeightChange={setComposerReserveHeight}
              isStreaming={isStreaming}
              disabled={!isConnected}
              thinkingLevel={thinkingLevel as ThinkingLevel}
              onThinkingChange={(level) => sessionStore.getState().setThinkingLevel(level)}
              permissionMode={permissionMode as PermissionMode}
              onPermissionModeToggle={() => sessionStore.getState().togglePermissionMode()}
              fastModeEnabled={fastModeEnabled}
              fastModeSupported={fastModeSupported}
              onFastModeToggle={() => sessionStore.getState().setFastModeEnabled(!fastModeStoreEnabled)}
              mode={mode}
              onModeToggle={() => sessionStore.getState().setMode(mode === 'build' ? 'plan' : 'build')}
              onModelSelect={(modelId) => void handleModelSelect(modelId)}
              model={model}
              models={models}
              sessionType={plane}
              workspaceDirectory={workspaceDirectory}
              targetBranch={workspaceTargetBranch}
              tokenCount={tokenCount}
              contentMaxWidth={920}
            />
          </View>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center' },
  topBar: {
    minHeight: 46,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: 12,
  },
  titleBlock: { flex: 1, minWidth: 0, gap: 2 },
  title: { fontSize: 14, fontWeight: '700', letterSpacing: -0.2 },
  meta: { fontSize: 12 },
  topActions: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  chip: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 7,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  iconChip: {
    width: 32,
    height: 32,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  canvas: { flex: 1 },
  column: { flex: 1, maxWidth: 1100, width: '100%', alignSelf: 'center' },
  composer: { position: 'relative', zIndex: 20 },
  empty: { alignItems: 'center', justifyContent: 'center', paddingHorizontal: 28, gap: 8 },
  emptyBody: { fontSize: 13, lineHeight: 19, textAlign: 'center', maxWidth: 420 },
});
