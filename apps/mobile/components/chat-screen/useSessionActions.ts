import { type MutableRefObject, useCallback, useEffect, useRef } from "react";
import { Alert, Platform } from "react-native";

import type { ModelInfo, SessionResponse, SessionType } from "@mitsuro/api";
import {
  type Attachment as SessionAttachment,
  beginMitsuroPerformanceSpan,
  resolveHiveSendTarget,
  type SendMessageOptions,
  type StopStreamingOptions,
} from "@mitsuro/state";
import type { useConnection } from "../../hooks/useConnection";
import type { useStores } from "../../hooks/useStores";
import * as Haptics from "../../platform/haptics";
import * as SecureStore from "../../platform/secure-store";
import {
  getWorkspaceMode,
  sessionTypeForTab,
  tabForSessionType,
  type WorkspaceMode,
} from "./helpers";
import {
  IDENTITY_STORAGE_KEYS,
  writeCanonicalAsyncValue,
} from "../../platform/identity-storage";
import {
  findCodeSessionForProject,
  type ResolvedSendIntent,
  resolveSendIntent,
} from "./sendIntent";
import { createSessionCreationCoordinator } from "./sessionCreationCoordinator";
import {
  createLatestIntentScheduler,
  type LatestIntentScheduler,
} from "../navigation/latestIntentScheduler";
import { runGenericSessionDeleteIfAllowed } from "./hiveSessionDeleteFence";
import {
  isCurrentSessionNavigationIntent,
  isCurrentSessionSendIntent,
} from "../navigation/sessionNavigationIntentFence";
import {
  beginAllModeSessionDeletionAdmission,
  clearDeletedSessionFromModeStoreGraphs,
  rollbackSessionDeletionAdmissions,
  runSessionDeletionBatch,
  type SessionDeletionAdmission,
} from "./sessionDeletionAdmission";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type ConnectionClient = ReturnType<typeof useConnection>["client"];

type SessionStoreApi = LoadedStores["session"];
type SessionsStoreApi = LoadedStores["sessions"];
type WorkspaceStoreApi = LoadedStores["workspace"];
type ModeStores = LoadedStores["modes"];
type SessionSelectionIntent = {
  id: string;
  sessionType: SessionType;
  force?: boolean;
};

function showActionMessage(title: string, message: string) {
  if (Platform.OS === "web") {
    if (typeof window !== "undefined") {
      window.alert(`${title}\n\n${message}`);
    }
    return;
  }
  Alert.alert(title, message);
}

function confirmDestructiveAction(
  title: string,
  message: string,
  onConfirm: () => void | Promise<void>,
) {
  if (Platform.OS === "web") {
    // react-native-web intentionally implements Alert.alert as a no-op. Use
    // the browser's real confirmation boundary so destructive callbacks are
    // reachable in the shared web preview.
    if (
      typeof window !== "undefined" && window.confirm(`${title}\n\n${message}`)
    ) {
      void onConfirm();
    }
    return;
  }

  Alert.alert(title, message, [
    { text: "Cancel", style: "cancel" },
    {
      text: "Delete",
      style: "destructive",
      onPress: () => void onConfirm(),
    },
  ]);
}

interface UseSessionActionsArgs {
  client: ConnectionClient;
  activeTab: number;
  activeToolCallId: string | null;
  setActiveToolCallId: (value: string | null) => void;
  setActiveTab: (value: number) => void;
  setDrawerOpen: (value: boolean) => void;
  ensureModelReady: (targetStore?: SessionStoreApi) => Promise<string | null>;
  sessionStore: SessionStoreApi;
  sessionsStore: SessionsStoreApi;
  workspace: WorkspaceStoreApi;
  modeStores: ModeStores;
  sessions: SessionResponse[];
  models: ModelInfo[];
  onSharedModelSelect?: (model: ModelInfo) => void;
  suppressCompletionRef: { current: boolean };
  lastSessionIdByTypeRef: MutableRefObject<Record<SessionType, string | null>>;
  navigationIntentGenerationRef: MutableRefObject<number>;
}

export function useSessionActions({
  client,
  activeTab,
  activeToolCallId,
  setActiveToolCallId,
  setActiveTab,
  setDrawerOpen,
  ensureModelReady,
  sessionStore,
  sessionsStore,
  workspace,
  modeStores,
  sessions,
  models,
  onSharedModelSelect,
  suppressCompletionRef,
  lastSessionIdByTypeRef,
  navigationIntentGenerationRef,
}: UseSessionActionsArgs) {
  const sessionCreationCoordinatorRef = useRef(
    createSessionCreationCoordinator<SessionResponse | null>(),
  );
  const admitSessionSelectionRef = useRef(
    ({ id, sessionType, force = false }: SessionSelectionIntent) => {
      const targetStore = modeStores[sessionType].session;
      if (!force && targetStore.getState().sessionId === id) return;
      void targetStore.getState().loadSession(id, force);
    },
  );
  admitSessionSelectionRef.current = ({ id, sessionType, force = false }) => {
    const targetStore = modeStores[sessionType].session;
    if (!force && targetStore.getState().sessionId === id) return;
    void targetStore.getState().loadSession(id, force);
  };
  const sessionSelectionSchedulerRef = useRef<
    LatestIntentScheduler<SessionSelectionIntent> | null
  >(null);
  if (!sessionSelectionSchedulerRef.current) {
    sessionSelectionSchedulerRef.current = createLatestIntentScheduler({
      // Loading a transcript is heavy work. Keep replacing the pending
      // selection while the user is still tapping and hydrate only the final
      // quiet destination.
      quietDelayMs: 72,
      onFlush: (intent) => admitSessionSelectionRef.current(intent),
    });
  }
  const deletionBoundaryActiveRef = useRef(true);
  const deletionModeStoresRef = useRef(modeStores);
  const deletionSessionsStoreRef = useRef(sessionsStore);
  deletionModeStoresRef.current = modeStores;
  deletionSessionsStoreRef.current = sessionsStore;
  useEffect(() => {
    deletionBoundaryActiveRef.current = true;
    const scheduler = sessionSelectionSchedulerRef.current;
    return () => {
      scheduler?.cancel();
      deletionBoundaryActiveRef.current = false;
    };
  }, []);
  const scheduleSessionSelection = useCallback(
    (intent: SessionSelectionIntent) => {
      sessionSelectionSchedulerRef.current?.submit(intent);
    },
    [],
  );
  const cancelPendingSessionSelection = useCallback(() => {
    sessionSelectionSchedulerRef.current?.cancel();
  }, []);
  const stopCurrentStream = useCallback(
    (
      suppressCompletion = true,
      stopOptions?: StopStreamingOptions,
    ) => {
      if (sessionStore.getState().isStreaming) {
        suppressCompletionRef.current = suppressCompletion;
        sessionStore.getState().stopStreaming(stopOptions);
      }
      setActiveToolCallId(null);
    },
    [sessionStore, setActiveToolCallId, suppressCompletionRef],
  );

  const detachCurrentSession = useCallback(() => {
    suppressCompletionRef.current = true;
    sessionStore.getState().detachSession();
    setActiveToolCallId(null);
  }, [
    sessionStore,
    setActiveToolCallId,
    suppressCompletionRef,
  ]);

  const bootstrapSession = useCallback(
    async (session: SessionResponse) => {
      const target = modeStores[session.session_type];
      const currentState = target.session.getState();
      const currentModel = currentState.model;
      const currentModelInfo = currentState.modelInfo;
      const currentThinkingLevel = currentState.thinkingLevel;
      const directory = session.project_dir ?? session.working_dir ?? null;
      const workspaceMode = (session.workspace_mode ??
        getWorkspaceMode(directory)) as WorkspaceMode;

      // Bind durable identity immediately so the empty shell is interactive.
      target.session
        .getState()
        .initSession(
          session.id,
          session.title || "",
          session.permission_mode,
          session.session_type,
        );
      target.workspace
        .getState()
        .initFromSession(
          session.id,
          directory,
          workspaceMode,
          session.target_branch ?? null,
        );

      if (currentModel) {
        const modelInfo = currentModelInfo ??
          models.find((candidate) => candidate.id === currentModel) ??
          null;
        // Keep create path local-first; setModel may persist, but we already
        // have a usable model for the empty composer.
        target.session
          .getState()
          .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
      }
      if (target.session.getState().thinkingLevel !== currentThinkingLevel) {
        target.session.getState().setThinkingLevel(currentThinkingLevel);
      }

      // Optimistic list patch + soft refresh. Never block New Chat on GET /sessions.
      sessionsStore.getState().upsertSession({
        id: session.id,
        title: session.title || "",
        updated_at: session.updated_at,
        token_count: session.token_count ?? null,
        parent_session_id: session.parent_session_id ?? null,
        working_dir: session.working_dir ?? null,
        project_dir: session.project_dir ?? null,
        workspace_mode: session.workspace_mode,
        session_type: session.session_type,
        target_branch: session.target_branch ?? null,
        permission_mode: session.permission_mode,
      });
      void sessionsStore.getState().loadSessions();
    },
    [modeStores, models, sessionsStore],
  );

  const createSessionForCurrentTab = useCallback(
    async (
      directory?: string,
      targetBranch?: string | null,
      requestedType?: SessionType,
    ) => {
      if (!client) {
        return null;
      }

      const targetType = requestedType ?? sessionTypeForTab(activeTab);
      const targetStore = modeStores[targetType].session;
      const creationCoordinator = sessionCreationCoordinatorRef.current;
      sessionSelectionSchedulerRef.current?.cancel();
      const finishShellSpan = beginMitsuroPerformanceSpan(
        "new_chat.shell",
        targetType,
      );

      // Instant local shell: close chrome and clear previous session without
      // waiting on network. Composer becomes empty/interactive immediately.
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(targetType));
      setActiveToolCallId(null);
      if (!creationCoordinator.hasPending(targetType)) {
        if (targetStore.getState().sessionId) {
          targetStore.getState().detachSession();
        }
        // Clear to a blank local draft shell before durable id arrives.
        targetStore.setState({
          sessionId: null,
          title: "",
          messages: [],
          isLoading: true,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          error: null,
          tokenCount: 0,
          queuedMessages: [],
        } as never);
      }
      finishShellSpan();

      try {
        const session = await creationCoordinator.run(
          targetType,
          async (isCurrent) => {
            const finishBindSpan = beginMitsuroPerformanceSpan(
              "new_chat.session_bind",
              targetType,
            );
            try {
              // Only hard-wait for a model when none is already usable.
              if (!targetStore.getState().model) {
                await ensureModelReady(targetStore);
              } else {
                void ensureModelReady(targetStore);
              }
              const created = await client.createSession(
                undefined,
                directory,
                targetBranch ?? undefined,
                directory ? "selected" : "neutral",
                targetType,
                targetStore.getState().permissionMode,
              );
              if (!isCurrent()) return null;
              await bootstrapSession(created);
              if (!isCurrent()) return null;
              lastSessionIdByTypeRef.current[created.session_type] = created.id;
              void Haptics.notificationAsync(
                Haptics.NotificationFeedbackType.Success,
              );
              return created;
            } finally {
              finishBindSpan();
            }
          },
        );
        return session;
      } catch {
        targetStore.setState({
          isLoading: false,
          error: "Failed to create session",
        } as never);
        return null;
      }
    },
    [
      activeTab,
      bootstrapSession,
      client,
      ensureModelReady,
      lastSessionIdByTypeRef,
      modeStores,
      setActiveToolCallId,
      setDrawerOpen,
      setActiveTab,
    ],
  );

  /**
   * Ensure the durable per-user Hive companion is the active session.
   * Used when opening the Hive tab, for New in hive mode, and as the send
   * fallback when no hive session is loaded yet.
   */
  const ensureHiveCompanionSession = useCallback(
    async (): Promise<string | null> => {
      if (!client) {
        return null;
      }
      return sessionStore.getState().ensureHiveMainSession();
    },
    [client, sessionStore],
  );

  const ensureSessionForSend = useCallback(
    async (): Promise<ResolvedSendIntent | null> => {
      // Hive never precreates ad-hoc sessions from the composer. Send to the
      // hive session that is already loaded (the durable companion or a Worker
      // DM); only ensure the companion when nothing is loaded yet.
      if (activeTab === 2 || sessionTypeForTab(activeTab) === "hive") {
        const hiveState = sessionStore.getState();
        const target = resolveHiveSendTarget({
          sessionId: hiveState.sessionId,
          sessionType: hiveState.sessionType,
        });
        if (target.kind === "ensure-companion") {
          const mainId = await ensureHiveCompanionSession();
          if (!mainId) {
            return null;
          }
        }
        return {
          shouldPrecreate: false,
          sendOptions: { sessionType: "hive" },
        };
      }

      const currentSessionId = sessionStore.getState().sessionId;
      const wsState = workspace.getState();
      const intent = resolveSendIntent({
        activeTab,
        currentSessionId,
        workspaceDirectory: wsState.directory,
        workspaceMode: wsState.mode,
        targetBranch: wsState.targetBranch,
      });

      if (!intent.shouldPrecreate) {
        return intent;
      }

      if (!client) {
        return null;
      }

      try {
        const precreate = intent.precreate;
        const targetType = precreate?.sessionType ??
          sessionTypeForTab(activeTab);
        const session = await sessionCreationCoordinatorRef.current.run(
          targetType,
          async (isCurrent) => {
            const finishBindSpan = beginMitsuroPerformanceSpan(
              "new_chat.session_bind",
              targetType,
            );
            try {
              const created = await client.createSession(
                undefined,
                precreate?.projectDir ?? undefined,
                precreate?.targetBranch ?? undefined,
                precreate?.workspaceMode,
                targetType,
                sessionStore.getState().permissionMode,
              );
              if (!isCurrent()) return null;
              await bootstrapSession(created);
              if (!isCurrent()) return null;
              lastSessionIdByTypeRef.current[created.session_type] = created.id;
              return created;
            } finally {
              finishBindSpan();
            }
          },
        );
        if (!session) {
          return null;
        }
        return { ...intent, sendOptions: undefined };
      } catch {
        return null;
      }
    },
    [
      activeTab,
      bootstrapSession,
      client,
      lastSessionIdByTypeRef,
      sessionStore,
      workspace,
    ],
  );

  const loadSession = useCallback(
    async (session: SessionResponse) => {
      sessionCreationCoordinatorRef.current.invalidate(session.session_type);
      // Close drawer and switch mode immediately so the gesture feels instant.
      lastSessionIdByTypeRef.current[session.session_type] = session.id;
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(session.session_type));
      // loadSession already detaches stream callbacks for the leaving session.
      // Avoid an extra detachSession() which can thrash presence/poll state.
      scheduleSessionSelection({
        id: session.id,
        sessionType: session.session_type,
      });
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
      scheduleSessionSelection,
      setActiveTab,
      setDrawerOpen,
    ],
  );

  const loadSessionById = useCallback(
    async (id: string) => {
      setDrawerOpen(false);
      const target = sessions.find((session) => session.id === id);
      const targetType = target?.session_type ?? "hive";
      sessionCreationCoordinatorRef.current.invalidate(targetType);
      lastSessionIdByTypeRef.current[targetType] = id;
      setActiveTab(tabForSessionType(targetType));
      scheduleSessionSelection({ id, sessionType: targetType });
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
      scheduleSessionSelection,
      sessions,
      setActiveTab,
      setDrawerOpen,
    ],
  );

  const openProjectInCode = useCallback(
    async (projectDir: string, targetBranch?: string | null) => {
      if (!client) {
        return;
      }

      setDrawerOpen(false);
      setActiveTab(1);
      const codeStore = modeStores.code.session;

      const existing = findCodeSessionForProject(
        sessions,
        projectDir,
        targetBranch ?? null,
      );

      if (existing) {
        sessionCreationCoordinatorRef.current.invalidate("code");
        lastSessionIdByTypeRef.current.code = existing.id;
        // loadSession detaches the previous stream attachment; avoid thrashing
        // presence/poll state with an extra detachSession on thread open.
        scheduleSessionSelection({ id: existing.id, sessionType: "code" });
        return;
      }

      sessionSelectionSchedulerRef.current?.cancel();

      try {
        const session = await sessionCreationCoordinatorRef.current.run(
          "code",
          async (isCurrent) => {
            await ensureModelReady(codeStore);
            const created = await client.createSession(
              undefined,
              projectDir,
              targetBranch ?? undefined,
              "selected",
              "code",
              codeStore.getState().permissionMode,
            );
            if (!isCurrent()) {
              return null;
            }
            await bootstrapSession(created);
            if (!isCurrent()) {
              return null;
            }
            lastSessionIdByTypeRef.current.code = created.id;
            setActiveToolCallId(null);
            void Haptics.notificationAsync(
              Haptics.NotificationFeedbackType.Success,
            );
            return created;
          },
        );
        if (!session) return;
      } catch {
        return;
      }
    },
    [
      bootstrapSession,
      client,
      ensureModelReady,
      modeStores,
      scheduleSessionSelection,
      sessions,
      setActiveTab,
      setActiveToolCallId,
      setDrawerOpen,
      lastSessionIdByTypeRef,
    ],
  );

  const handleNewSession = useCallback(async (sessionType?: SessionType) => {
    // Hive has one durable companion — never create a fresh peer chat from "new".
    if (sessionType === "hive" || (sessionType == null && activeTab === 2)) {
      const navigationIntentGeneration = navigationIntentGenerationRef.current;
      const ensuredSessionId = await ensureHiveCompanionSession();
      if (!ensuredSessionId) return;
      if (
        !isCurrentSessionNavigationIntent(
          navigationIntentGeneration,
          navigationIntentGenerationRef.current,
          ensuredSessionId,
          modeStores.hive.session.getState().sessionId,
        )
      ) return;
      setActiveTab(2);
      setDrawerOpen(false);
      return;
    }
    await createSessionForCurrentTab(undefined, undefined, sessionType);
  }, [
    activeTab,
    createSessionForCurrentTab,
    ensureHiveCompanionSession,
    modeStores,
    navigationIntentGenerationRef,
    setActiveTab,
    setDrawerOpen,
  ]);

  const handleDirectorySelected = useCallback(
    async (path: string) => {
      await createSessionForCurrentTab(path, undefined, "code");
    },
    [createSessionForCurrentTab],
  );

  const handleDeleteSession = useCallback(
    (
      id: string,
      sessionType: SessionType,
      onDeleted?: () => void,
      onFailed?: () => void,
    ) => {
      void runGenericSessionDeleteIfAllowed(
        {
          sessionId: id,
          sessionType,
          resolveHiveBinding: client
            ? (sessionId) => client.getHiveWorkerBySession(sessionId)
            : undefined,
        },
        () => {
          confirmDestructiveAction(
            "Delete Session",
            "Delete this session?",
            async () => {
              const isCurrentDeletionBoundary = () =>
                deletionBoundaryActiveRef.current &&
                deletionModeStoresRef.current === modeStores &&
                deletionSessionsStoreRef.current === sessionsStore;
              if (!isCurrentDeletionBoundary()) return;
              let admission: SessionDeletionAdmission;
              try {
                admission = await beginAllModeSessionDeletionAdmission(
                  modeStores,
                  id,
                );
              } catch (scrubError) {
                if (!isCurrentDeletionBoundary()) return;
                showActionMessage(
                  "Delete unavailable",
                  scrubError instanceof Error
                    ? scrubError.message
                    : "Queued recovery could not be cleared safely.",
                );
                return;
              }
              if (!isCurrentDeletionBoundary()) {
                try {
                  await admission.rollback();
                } catch (rollbackError) {
                  console.error(
                    "Failed to restore session recovery after the deletion boundary changed.",
                    rollbackError,
                  );
                }
                return;
              }
              try {
                onDeleted?.();
              } catch (optimisticError) {
                let deleteError: unknown = optimisticError;
                try {
                  await admission.rollback();
                } catch (rollbackError) {
                  deleteError = rollbackError;
                }
                onFailed?.();
                if (isCurrentDeletionBoundary()) {
                  showActionMessage(
                    "Delete unavailable",
                    deleteError instanceof Error
                      ? deleteError.message
                      : "The conversation list could not be updated safely.",
                  );
                }
                return;
              }

              let deleted = false;
              let deleteError: unknown = null;
              try {
                deleted = await sessionsStore.getState().deleteSession(
                  id,
                );
              } catch (error) {
                deleteError = error;
              }
              if (deleted) {
                const cleared = clearDeletedSessionFromModeStoreGraphs(
                  modeStores,
                  deletionModeStoresRef.current,
                  id,
                );
                admission.commit();
                if (cleared) setActiveToolCallId(null);
                if (!isCurrentDeletionBoundary()) {
                  // Remove only the optimistic override installed above. The
                  // replacement graph now owns its own session projection.
                  onFailed?.();
                  return;
                }
                void sessionsStore.getState().loadSessions();
                return;
              }

              try {
                await admission.rollback();
              } catch (rollbackError) {
                deleteError = rollbackError;
              }
              onFailed?.();
              if (deleteError && isCurrentDeletionBoundary()) {
                showActionMessage(
                  "Delete incomplete",
                  deleteError instanceof Error
                    ? deleteError.message
                    : "Session recovery could not be restored safely.",
                );
              }
              return;
            },
          );
        },
      );
    },
    [client, modeStores, sessionsStore, setActiveToolCallId],
  );

  const handleSetSessionPinned = useCallback(
    async (id: string, pinned: boolean): Promise<boolean> => {
      if (!client) return false;
      try {
        const updated = await client.updateSession(id, { pinned });
        sessionsStore.getState().upsertSession(updated);
        return true;
      } catch {
        showActionMessage(
          pinned ? "Couldn’t pin conversation" : "Couldn’t unpin conversation",
          "The conversation was not changed. Please try again.",
        );
        return false;
      }
    },
    [client, sessionsStore],
  );

  const handleSetSessionArchived = useCallback(
    async (id: string, archived: boolean): Promise<boolean> => {
      if (!client) return false;
      const previous =
        sessionsStore.getState().sessions.find((session) =>
          session.id === id
        ) ?? null;
      sessionsStore.getState().setSessionArchived(id, archived);
      try {
        const updated = await client.updateSession(id, { archived });
        sessionsStore.getState().upsertSession(updated);
        return true;
      } catch {
        if (previous) {
          sessionsStore.getState().revertSession(previous);
        }
        showActionMessage(
          archived
            ? "Couldn’t archive conversation"
            : "Couldn’t restore conversation",
          "The conversation was not changed. Please try again.",
        );
        return false;
      }
    },
    [client, sessionsStore],
  );

  const handleSetProjectPinned = useCallback(
    async (ids: string[], pinned: boolean): Promise<boolean> => {
      if (!client || ids.length === 0) return false;
      try {
        const updated = await Promise.all(
          ids.map((id) => client.updateSession(id, { pinned })),
        );
        for (const session of updated) {
          sessionsStore.getState().upsertSession(session);
        }
        return true;
      } catch {
        await sessionsStore.getState().loadSessions();
        showActionMessage(
          pinned ? "Couldn’t pin project" : "Couldn’t unpin project",
          "Some conversations may not have changed. The list has been refreshed.",
        );
        return false;
      }
    },
    [client, sessionsStore],
  );

  const handleSetProjectArchived = useCallback(
    async (ids: string[], archived: boolean): Promise<boolean> => {
      if (!client || ids.length === 0) return false;
      const previous = ids
        .map((id) =>
          sessionsStore.getState().sessions.find((session) =>
            session.id === id
          ) ?? null
        )
        .filter((session): session is NonNullable<typeof session> =>
          session != null
        );
      for (const id of ids) {
        sessionsStore.getState().setSessionArchived(id, archived);
      }
      try {
        const updated = await Promise.all(
          ids.map((id) => client.updateSession(id, { archived })),
        );
        for (const session of updated) {
          sessionsStore.getState().upsertSession(session);
        }
        return true;
      } catch {
        for (const session of previous) {
          sessionsStore.getState().revertSession(session);
        }
        await sessionsStore.getState().loadSessions();
        showActionMessage(
          archived ? "Couldn’t archive project" : "Couldn’t restore project",
          "Some conversations may not have changed. The list has been refreshed.",
        );
        return false;
      }
    },
    [client, sessionsStore],
  );

  const handleDeleteProjectSessions = useCallback(
    (
      projectName: string,
      ids: string[],
      onDeleted?: () => void,
      onFailed?: (failedIds: string[]) => void,
    ) => {
      const deletionIds = [...new Set(ids)];
      if (deletionIds.length === 0) return;
      confirmDestructiveAction(
        "Delete project conversations?",
        `Delete ${deletionIds.length} ${
          deletionIds.length === 1 ? "conversation" : "conversations"
        } from ${projectName}? The project folder and its files will not be deleted.`,
        async () => {
          const isCurrentDeletionBoundary = () =>
            deletionBoundaryActiveRef.current &&
            deletionModeStoresRef.current === modeStores &&
            deletionSessionsStoreRef.current === sessionsStore;
          if (!isCurrentDeletionBoundary()) return;
          const admissionResults = await Promise.allSettled(
            deletionIds.map((id) =>
              beginAllModeSessionDeletionAdmission(modeStores, id)
            ),
          );
          const admissions = new Map<string, SessionDeletionAdmission>();
          admissionResults.forEach((result, index) => {
            if (result.status === "fulfilled") {
              admissions.set(deletionIds[index], result.value);
            }
          });
          const rollbackOutstandingAdmissions = async (): Promise<
            unknown | null
          > => {
            try {
              await rollbackSessionDeletionAdmissions(admissions);
              admissions.clear();
              return null;
            } catch (rollbackError) {
              // Core retains admission and requires a later begin to finish
              // the failed restore before it may acquire a fresh transport
              // lease. Keep the unresolved entries intact for this result.
              return rollbackError;
            }
          };
          if (
            admissionResults.some((result) => result.status === "rejected")
          ) {
            const admissionError = admissionResults.find((result) =>
              result.status === "rejected"
            );
            const rollbackError = await rollbackOutstandingAdmissions();
            if (isCurrentDeletionBoundary()) {
              showActionMessage(
                "Delete unavailable",
                rollbackError instanceof Error
                  ? rollbackError.message
                  : admissionError?.status === "rejected" &&
                      admissionError.reason instanceof Error
                  ? admissionError.reason.message
                  : "Queued recovery could not be cleared safely. No conversations were deleted.",
              );
            }
            return;
          }
          if (!isCurrentDeletionBoundary()) {
            const rollbackError = await rollbackOutstandingAdmissions();
            if (rollbackError) {
              console.error(
                "Failed to restore project recovery after the deletion boundary changed.",
                rollbackError,
              );
            }
            return;
          }
          try {
            onDeleted?.();
          } catch (optimisticError) {
            const rollbackError = await rollbackOutstandingAdmissions();
            onFailed?.(deletionIds);
            if (isCurrentDeletionBoundary()) {
              showActionMessage(
                "Delete unavailable",
                rollbackError instanceof Error
                  ? rollbackError.message
                  : optimisticError instanceof Error
                  ? optimisticError.message
                  : "The conversation list could not be updated safely.",
              );
            }
            return;
          }

          const result = await runSessionDeletionBatch(
            deletionIds,
            admissions,
            (id) => sessionsStore.getState().deleteSession(id),
            isCurrentDeletionBoundary,
            (id) => {
              if (
                clearDeletedSessionFromModeStoreGraphs(
                  modeStores,
                  deletionModeStoresRef.current,
                  id,
                )
              ) {
                setActiveToolCallId(null);
              }
            },
          );
          if (result.boundaryChanged) {
            // Undo this operation's stale optimistic overrides. The replacement
            // graph must project its own account/session list from scratch.
            onFailed?.(deletionIds);
            if (result.error) {
              console.error(
                "Failed to restore project recovery after the deletion boundary changed.",
                result.error,
              );
            }
            return;
          }
          if (!isCurrentDeletionBoundary()) {
            // The batch completed against the captured graph, but a replacement
            // became current before this continuation resumed. Remove every
            // operation-owned override so it cannot hide the replacement UI.
            onFailed?.(deletionIds);
            return;
          }
          if (result.remainingIds.length > 0) {
            onFailed?.(result.remainingIds);
          }
          await sessionsStore.getState().loadSessions();
          if (
            isCurrentDeletionBoundary() && result.remainingIds.length > 0
          ) {
            showActionMessage(
              "Some conversations weren’t deleted",
              result.error instanceof Error
                ? result.error.message
                : `${result.remainingIds.length} ${
                  result.remainingIds.length === 1
                    ? "conversation remains"
                    : "conversations remain"
                }. No further conversations were deleted.`,
            );
          }
        },
      );
    },
    [modeStores, sessionsStore, setActiveToolCallId],
  );

  const handleInteractiveToolResult = useCallback(
    async (targetSessionId: string, toolCallId: string, result: string) => {
      const currentSessionId = sessionStore.getState().sessionId;
      if (
        !currentSessionId || currentSessionId !== targetSessionId ||
        activeToolCallId
      ) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        await sessionStore.getState().submitToolResult(toolCallId, result);
      } catch {
        if (sessionStore.getState().sessionId === targetSessionId) {
          await sessionStore.getState().loadSession(targetSessionId, true);
        }
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, sessionStore, setActiveToolCallId],
  );

  const handlePlanConfirm = useCallback(
    async (
      targetSessionId: string,
      toolCallId: string,
      choice: "execute" | "abandon",
    ) => {
      if (sessionStore.getState().sessionId !== targetSessionId) return;
      if (choice === "execute") {
        sessionStore.getState().setMode("build");
      }
      await handleInteractiveToolResult(
        targetSessionId,
        toolCallId,
        JSON.stringify({ choice }),
      );
    },
    [handleInteractiveToolResult, sessionStore],
  );

  const handleSend = useCallback(
    async (
      content: string,
      attachments: SessionAttachment[] = [],
      targetFence?: {
        assertCurrent: () => void;
        skipModelReadiness?: boolean;
        rethrowErrors?: boolean;
        sendOptions?: Partial<SendMessageOptions>;
      },
    ) => {
      const trimmed = content.trim();
      if (!client || (!trimmed && attachments.length === 0)) {
        return;
      }

      const originatingNavigationIntentGeneration =
        navigationIntentGenerationRef.current;
      const originatingSessionId = sessionStore.getState().sessionId;
      const isCurrentOriginatingSelection = () =>
        targetFence != null ||
        isCurrentSessionNavigationIntent(
          originatingNavigationIntentGeneration,
          navigationIntentGenerationRef.current,
          originatingSessionId,
          sessionStore.getState().sessionId,
        );

      targetFence?.assertCurrent();
      if (!targetFence?.skipModelReadiness) {
        const resolvedModel = await ensureModelReady();
        targetFence?.assertCurrent();
        if (!isCurrentOriginatingSelection()) return;
        if (!resolvedModel) {
          sessionStore.setState({
            error:
              "No model is available yet. Check your model settings and try again.",
          });
          return;
        }
      }

      const sendIntent = await ensureSessionForSend();
      targetFence?.assertCurrent();
      if (!sendIntent) {
        return;
      }
      const transportSessionId = sessionStore.getState().sessionId;
      const isCurrentSendTarget = () =>
        targetFence != null ||
        isCurrentSessionSendIntent(
          originatingNavigationIntentGeneration,
          navigationIntentGenerationRef.current,
          originatingSessionId,
          transportSessionId,
          sessionStore.getState().sessionId,
        );
      if (!isCurrentSendTarget()) return;

      try {
        const exactSendOptions = targetFence?.sendOptions
          ? { ...sendIntent.sendOptions, ...targetFence.sendOptions }
          : sendIntent.sendOptions;
        await sessionStore
          .getState()
          .sendMessage(
            trimmed,
            attachments,
            exactSendOptions,
          );
      } catch (err) {
        if (targetFence) {
          try {
            targetFence.assertCurrent();
          } catch {
            // The originating Worker is no longer active. Let its composer
            // restore the draft without projecting A's error onto B.
            throw err;
          }
        } else if (!isCurrentSendTarget()) {
          // Generic sends historically resolve their composer promise after a
          // failure. Preserve that behavior, but never project A's late error
          // onto the newer session that now owns this store.
          return;
        }
        sessionStore.setState({
          error: err instanceof Error ? err.message : "Failed to send message.",
        });
        if (targetFence?.rethrowErrors) throw err;
      }
    },
    [
      activeTab,
      client,
      ensureModelReady,
      ensureSessionForSend,
      navigationIntentGenerationRef,
      sessionStore,
      workspace,
    ],
  );

  const handleModelSelect = useCallback(
    (modelInfo: ModelInfo) => {
      if (sessionStore === modeStores.hive.session) return;
      const modelId = modelInfo.id;
      onSharedModelSelect?.(modelInfo);
      sessionStore.setState({ error: null });
      sessionStore
        .getState()
        .setModel(modelId, modelInfo.provider ?? null, modelInfo);
      void writeCanonicalAsyncValue(
        SecureStore,
        IDENTITY_STORAGE_KEYS.selectedModel,
        modelId,
      );
    },
    [modeStores.hive.session, onSharedModelSelect, sessionStore],
  );

  const handleFastModeToggle = useCallback(() => {
    const currentModel = sessionStore.getState().model;
    if (currentModel) {
      const modelInfo = sessionStore.getState().modelInfo ??
        models.find((candidate) => candidate.id === currentModel) ??
        null;
      sessionStore
        .getState()
        .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
    }
    sessionStore.getState().toggleFastMode();
  }, [models, sessionStore]);

  return {
    stopCurrentStream,
    detachCurrentSession,
    loadSession,
    loadSessionById,
    openProjectInCode,
    ensureHiveCompanionSession,
    handleNewSession,
    handleDirectorySelected,
    handleDeleteSession,
    handleSetSessionPinned,
    handleSetSessionArchived,
    handleSetProjectPinned,
    handleSetProjectArchived,
    handleDeleteProjectSessions,
    handleInteractiveToolResult,
    handlePlanConfirm,
    handleSend,
    handleModelSelect,
    handleFastModeToggle,
    cancelPendingSessionSelection,
  };
}
