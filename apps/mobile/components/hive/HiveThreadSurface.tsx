import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { useThemeContext } from "../../hooks/useTheme";
import type { HiveChatContext } from "./types";
import { useHiveSessionView } from "./hooks/useHiveSessionView";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";
import { useActiveHiveWorkerBinding } from "./hooks/useActiveHiveWorkerBinding";
import {
  canShowHiveWorkerGoalForIntroduction,
  useActiveHiveWorkerIntroduction,
} from "./hooks/useActiveHiveWorkerIntroduction";
import { useHiveWorkerGoal } from "./hooks/useHiveWorkerGoal";
import { HiveWorkerIntroductionSheet } from "./HiveWorkerIntroductionSheet";
import { HiveWorkerGoalTracker } from "./HiveWorkerGoalTracker";
import {
  HiveWorkerComposer,
  HiveWorkerDirectChatHeader,
} from "./HiveWorkerDirectChat";

interface HiveThreadSurfaceProps {
  chat: HiveChatContext;
  workers: HiveWorkersState;
  scrollToMessageId?: string | null;
  onScrollTargetHandled?: () => void;
  showComposer?: boolean;
  externalBottomPadding?: number;
}

export function HiveThreadSurface({
  chat,
  workers,
  scrollToMessageId,
  onScrollTargetHandled,
  showComposer = true,
  externalBottomPadding = 150,
}: HiveThreadSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);
  const [introductionReserveHeight, setIntroductionReserveHeight] = useState(0);
  const [goalTrackerReserveHeight, setGoalTrackerReserveHeight] = useState(0);
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);
  const sessionView = useHiveSessionView();
  const workerBinding = useActiveHiveWorkerBinding(
    workers,
    sessionView.sessionId,
  );
  const activeWorker = workerBinding.worker;
  const tail = sessionView.messages.at(-1);
  const introduction = useActiveHiveWorkerIntroduction({
    workers,
    worker: activeWorker,
    sessionId: sessionView.sessionId,
    transcriptTailKey: `${sessionView.messages.length}:${tail?.id ?? "none"}:${
      tail?.role ?? "none"
    }`,
    isStreaming: sessionView.isStreaming,
  });
  const workerGoal = useHiveWorkerGoal({
    worker: activeWorker,
    sessionId: workerBinding.kind === "worker_dm"
      ? sessionView.sessionId
      : null,
    transcriptTailKey: `${sessionView.messages.length}:${tail?.id ?? "none"}:${
      tail?.role ?? "none"
    }`,
    isStreaming: sessionView.isStreaming,
  });
  const introductionStatus = introduction.introduction?.status;
  const introductionAllowsGoalTracker = canShowHiveWorkerGoalForIntroduction(
    introduction.detail,
  );
  const introductionDisablesComposer = introductionStatus === "queued" ||
    introductionStatus === "running" ||
    introductionStatus === "review_ready" ||
    introductionStatus === "failed" ||
    introductionStatus === "needs_recovery";
  const workerDisablesComposer = activeWorker?.status === "paused" ||
    activeWorker?.status === "archived";
  const introductionWorkerDisablesComposer =
    introduction.worker?.id === activeWorker?.id &&
    (introduction.worker?.status === "paused" ||
      introduction.worker?.status === "archived");
  const introductionIsResolving = Boolean(
    activeWorker &&
      (introduction.worker?.id !== activeWorker.id ||
        introduction.detail?.id !== activeWorker.id ||
        introduction.isLoading),
  );
  const introductionDetailError = activeWorker && !introduction.detail &&
      !introduction.isLoading
    ? introduction.error
    : null;
  const queuedRecoveryBlocked = sessionView.queuedRecoveryBlocked;
  const canResolveQueuedRecovery = queuedRecoveryBlocked &&
    sessionView.sessionId !== null;
  const surfaceError = sessionView.error ??
    (queuedRecoveryBlocked
      ? "Delivery status is uncertain. Retry or discard this queued message before sending another."
      : workerBinding.error ?? introductionDetailError);
  const showPrimaryHiveComposer = workerBinding.kind === "primary_hive" ||
    workerBinding.kind === "none";

  return (
    <View style={styles.container}>
      {activeWorker
        ? (
          <HiveWorkerDirectChatHeader
            worker={activeWorker}
            models={chat.models}
          />
        )
        : null}
      <ChatTranscript
        messages={sessionView.messages}
        sessionId={sessionView.sessionId}
        sessionType="hive"
        scrollStateKey={`hive:${sessionView.sessionId ?? "new"}`}
        isStreaming={sessionView.isStreaming}
        isThinking={sessionView.isThinking}
        isLoading={sessionView.isLoading}
        activeToolCallId={chat.activeToolCallId}
        onApproveTool={chat.onApproveTool}
        onDenyTool={chat.onDenyTool}
        onSubmitToolResult={chat.onSubmitToolResult}
        onPlanConfirm={workerBinding.kind === "primary_hive"
          ? chat.onPlanConfirm
          : undefined}
        bottomPadding={showComposer
          ? composerReserveHeight + introductionReserveHeight +
            goalTrackerReserveHeight + (goalTrackerReserveHeight > 0 ? 8 : 0)
          : externalBottomPadding + goalTrackerReserveHeight +
            (goalTrackerReserveHeight > 0 ? 8 : 0)}
        hideJumpToLatest={bottomControlsOpen}
        scrollToMessageId={scrollToMessageId}
        onScrollTargetHandled={onScrollTargetHandled}
        emptyState={
          <View style={styles.emptyState}>
            <Text
              style={[styles.emptyTitle, { color: t.foreground }]}
            >
              {sessionView.isStreaming || sessionView.isThinking
                ? "Starting conversation…"
                : "No messages yet"}
            </Text>
            <Text
              style={[styles.emptyBody, { color: t.mutedForeground }]}
            >
              {sessionView.isStreaming || sessionView.isThinking
                ? "Your Worker is preparing its first message."
                : "Send a message when you are ready."}
            </Text>
          </View>
        }
      />

      {workerBinding.kind === "worker_dm" && activeWorker &&
          activeWorker.status === "active" && introductionAllowsGoalTracker &&
          sessionView.sessionId
        ? (
          <HiveWorkerGoalTracker
            key={`${activeWorker.id}:${sessionView.sessionId}`}
            state={workerGoal}
            bottom={(showComposer
              ? composerReserveHeight + introductionReserveHeight
              : externalBottomPadding) + 8}
            onHeightChange={setGoalTrackerReserveHeight}
          />
        )
        : null}

      {surfaceError
        ? (
          <View
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            style={[
              styles.errorBanner,
              {
                bottom:
                  (showComposer
                    ? composerReserveHeight
                    : externalBottomPadding) + 8,
                borderColor: `${t.error}40`,
                backgroundColor: `${t.error}14`,
              },
            ]}
          >
            <Text style={[styles.errorText, { color: t.error }]}>
              {surfaceError}
            </Text>
            {queuedRecoveryBlocked
              ? (
                <View style={styles.errorActions}>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Retry uncertain queued message"
                    accessibilityHint="Attempts to deliver the queued message again"
                    accessibilityState={{
                      disabled: !canResolveQueuedRecovery,
                    }}
                    disabled={!canResolveQueuedRecovery}
                    onPress={() => {
                      const targetSessionId = sessionView.sessionId;
                      if (!targetSessionId) return;
                      void sessionView.retryQueuedRecovery(targetSessionId);
                    }}
                    style={({ pressed }) => [
                      styles.errorAction,
                      {
                        borderColor: `${t.error}70`,
                        opacity: !canResolveQueuedRecovery
                          ? 0.45
                          : pressed
                          ? 0.7
                          : 1,
                      },
                    ]}
                  >
                    <Text
                      style={[styles.errorActionText, { color: t.error }]}
                    >
                      Retry
                    </Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Discard uncertain queued message"
                    accessibilityHint="Removes the queued message without sending it"
                    accessibilityState={{
                      disabled: !canResolveQueuedRecovery,
                    }}
                    disabled={!canResolveQueuedRecovery}
                    onPress={() => {
                      const targetSessionId = sessionView.sessionId;
                      if (!targetSessionId) return;
                      void sessionView.discardQueuedRecovery(targetSessionId);
                    }}
                    style={({ pressed }) => [
                      styles.errorAction,
                      {
                        borderColor: `${t.error}70`,
                        opacity: !canResolveQueuedRecovery
                          ? 0.45
                          : pressed
                          ? 0.7
                          : 1,
                      },
                    ]}
                  >
                    <Text
                      style={[styles.errorActionText, { color: t.error }]}
                    >
                      Discard
                    </Text>
                  </Pressable>
                </View>
              )
              : (workerBinding.error && !workerBinding.isResolving) ||
                  introductionDetailError
              ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={workerBinding.error
                    ? "Retry Hive conversation binding"
                    : "Retry Worker details"}
                  onPress={workerBinding.error
                    ? workerBinding.retry
                    : introduction.refresh}
                  style={styles.errorRetry}
                >
                  <Text
                    style={[styles.errorRetryText, { color: t.userMessage }]}
                  >
                    Retry
                  </Text>
                </Pressable>
              )
              : null}
          </View>
        )
        : null}

      {showComposer
        ? (
          <>
            <HiveWorkerIntroductionSheet
              state={introduction}
              bottom={composerReserveHeight}
              onHeightChange={setIntroductionReserveHeight}
            />
            {activeWorker && sessionView.sessionId
              ? (
                <HiveWorkerComposer
                  key={`${activeWorker.id}:${sessionView.sessionId}`}
                  worker={activeWorker}
                  sessionId={sessionView.sessionId}
                  onSend={chat.onWorkerSend}
                  onStop={chat.onWorkerStop}
                  onHeightChange={setComposerReserveHeight}
                  isStreaming={chat.isStreaming}
                  disabled={workerDisablesComposer ||
                    introductionWorkerDisablesComposer ||
                    introductionDisablesComposer ||
                    introductionIsResolving ||
                    queuedRecoveryBlocked ||
                    (sessionView.messages.length === 0 &&
                      (sessionView.isStreaming || sessionView.isThinking))}
                />
              )
              : workerBinding.isResolving
              ? null
              : showPrimaryHiveComposer
              ? (
                <ChatBar
                  draftKey="hive"
                  onSend={chat.onSend}
                  onStop={chat.onStop}
                  onHeightChange={setComposerReserveHeight}
                  isStreaming={chat.isStreaming}
                  disabled={!sessionView.sessionId ||
                    queuedRecoveryBlocked ||
                    (sessionView.messages.length === 0 &&
                      (sessionView.isStreaming || sessionView.isThinking))}
                  thinkingLevel={chat.thinkingLevel}
                  onThinkingChange={chat.onThinkingChange}
                  permissionMode={chat.permissionMode}
                  onPermissionModeToggle={chat.onPermissionModeToggle}
                  fastModeEnabled={chat.fastModeEnabled}
                  fastModeSupported={chat.fastModeSupported}
                  onFastModeToggle={chat.onFastModeToggle}
                  mode={chat.mode}
                  onModeToggle={chat.onModeToggle}
                  onModelSelect={chat.onModelSelect}
                  model={chat.model ?? null}
                  modelKey={chat.modelKey}
                  models={chat.models}
                  sessionType="hive"
                  tokenCount={chat.tokenCount}
                  onOverlayOpenChange={setBottomControlsOpen}
                />
              )
              : null}
          </>
        )
        : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    minHeight: 0,
  },
  errorBanner: {
    position: "absolute",
    left: 16,
    right: 16,
    zIndex: 30,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  errorText: {
    flex: 1,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
  errorRetry: {
    minHeight: 44,
    justifyContent: "center",
    paddingHorizontal: 6,
  },
  errorRetryText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "700",
  },
  errorActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  errorAction: {
    minHeight: 44,
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 10,
  },
  errorActionText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "700",
  },
  emptyState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 32,
  },
  emptyTitle: {
    fontSize: 17,
    lineHeight: 24,
    fontWeight: "600",
    textAlign: "center",
  },
  emptyBody: {
    marginTop: 6,
    fontSize: 14,
    lineHeight: 20,
    textAlign: "center",
  },
});
