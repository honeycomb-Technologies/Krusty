import { useEffect, useMemo, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';
import { HiveScreen } from '@mobile/components/hive/HiveScreen';
import type { HiveChatContext, HiveTopLevelView } from '@mobile/components/hive/types';
import type { ModelInfo } from '@mitsuro/api';
import type { PermissionMode, ThinkingLevel } from '@mitsuro/state';
import { resolveUsableModel } from '@mitsuro/state';
import { useConnection } from '@mobile/hooks/useConnection';
import {
  useSessionStore,
  useStores,
  useWorkspaceStore,
} from '@mobile/hooks/useStores';
import { useThemeContext } from '@mobile/hooks/useTheme';
import * as SecureStore from '@mobile/platform/secure-store';

const SELECTED_MODEL_KEY = 'mitsuro:selected-model';

export function HivePlane({
  onOpenProject,
}: {
  onOpenProject?: (path: string, branch?: string | null) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const { client, isConnected } = useConnection();
  const stores = useStores();
  const sessionStore = stores?.modes.hive.session;
  const [topLevel, setTopLevel] = useState<HiveTopLevelView>('hive');
  const [models, setModels] = useState<ModelInfo[]>([]);

  const sessionId = useSessionStore((state) => state.sessionId, 'hive') ?? null;
  const title = useSessionStore((state) => state.title, 'hive') ?? null;
  const messages = useSessionStore((state) => state.messages, 'hive') ?? [];
  const error = useSessionStore((state) => state.error, 'hive') ?? null;
  const isLoading = useSessionStore((state) => state.isLoading, 'hive') ?? false;
  const isStreaming = useSessionStore((state) => state.isStreaming, 'hive') ?? false;
  const isThinking = useSessionStore((state) => state.isThinking, 'hive') ?? false;
  const thinkingLevel = useSessionStore((state) => state.thinkingLevel, 'hive') ?? 'medium';
  const permissionMode = useSessionStore((state) => state.permissionMode, 'hive') ?? 'autonomous';
  const fastModeEnabled = useSessionStore((state) => state.fastModeEnabled, 'hive') ?? false;
  const mode = useSessionStore((state) => state.mode, 'hive') ?? 'build';
  const model = useSessionStore((state) => state.model, 'hive') ?? null;
  const tokenCount = useSessionStore((state) => state.tokenCount, 'hive') ?? 0;
  const workspaceDirectory = useWorkspaceStore((state) => state.directory, 'hive') ?? null;

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
        // catalog optional for Hive surface boot
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, isConnected, sessionStore]);

  const chat: HiveChatContext = useMemo(
    () => ({
      sessionId,
      title,
      messages,
      error,
      isLoading,
      isStreaming,
      isThinking,
      activeToolCallId: null,
      thinkingLevel: thinkingLevel as ThinkingLevel,
      permissionMode: permissionMode as PermissionMode,
      fastModeEnabled,
      fastModeSupported: false,
      mode,
      model,
      models,
      tokenCount,
      onApproveTool: (_targetSessionId, toolCallId) => {
        void sessionStore?.getState().submitToolApproval(toolCallId, true);
      },
      onDenyTool: (_targetSessionId, toolCallId) => {
        void sessionStore?.getState().submitToolApproval(toolCallId, false);
      },
      onSubmitToolResult: (toolCallId, result) =>
        sessionStore?.getState().submitToolResult(toolCallId, result),
      onPlanConfirm: (toolCallId, choice) =>
        sessionStore
          ?.getState()
          .submitToolResult(toolCallId, JSON.stringify({ choice })),
      onSend: async (content, attachments) => {
        if (!sessionStore) return;
        if (!sessionStore.getState().sessionId && client) {
          const created = await client.createSession(
            undefined,
            undefined,
            undefined,
            'neutral',
            'hive',
            sessionStore.getState().permissionMode,
          );
          sessionStore.getState().initSession(
            created.id,
            created.title || '',
            created.permission_mode,
            created.session_type,
          );
          await stores?.sessions.getState().loadSessions();
        }
        const mapped = (attachments ?? []).map((attachment) => ({
          type: attachment.type,
          name: attachment.name ?? 'attachment',
          uri: attachment.uri,
          base64: attachment.base64,
          mimeType: attachment.mimeType ?? 'application/octet-stream',
        }));
        await sessionStore.getState().sendMessage(content, mapped, {
          sessionType: 'hive',
        });
      },
      onStop: () => sessionStore?.getState().stopStreaming(),
      onThinkingChange: (level) => sessionStore?.getState().setThinkingLevel(level),
      onPermissionModeToggle: () => sessionStore?.getState().togglePermissionMode(),
      onFastModeToggle: () => {},
      onModeToggle: () =>
        sessionStore?.getState().setMode(mode === 'build' ? 'plan' : 'build'),
      onModelSelect: (modelId) => sessionStore?.getState().setModel(modelId),
    }),
    [
      client,
      error,
      fastModeEnabled,
      isLoading,
      isStreaming,
      isThinking,
      messages,
      mode,
      model,
      models,
      permissionMode,
      sessionId,
      sessionStore,
      stores,
      thinkingLevel,
      title,
      tokenCount,
    ],
  );

  if (!stores || !sessionStore) {
    return (
      <View style={[styles.center, { backgroundColor: t.background }]}>
        <Text style={{ color: t.mutedForeground }}>Loading Hive…</Text>
      </View>
    );
  }

  return (
    <View style={[styles.root, { backgroundColor: t.background }]}>
      <HiveScreen
        workspaceDirectory={workspaceDirectory}
        requestedTopLevel={topLevel}
        chat={chat}
        onOpenRunById={async (runId) => {
          await sessionStore.getState().loadSession(runId);
        }}
        onOpenProject={onOpenProject}
        onDeleteRun={(runId) => {
          void stores.sessions.getState().deleteSession(runId);
        }}
        onTopLevelChange={setTopLevel}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center' },
});
