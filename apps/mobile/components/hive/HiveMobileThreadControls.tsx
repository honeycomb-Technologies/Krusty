import { memo, type ReactNode } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { ActiveConversationActivity } from "../chat-screen/ActiveConversationSurface";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveWorkerComposer } from "./HiveWorkerDirectChat";
import { HiveWorkerGoalTracker } from "./HiveWorkerGoalTracker";
import { HiveWorkerIntroductionSheet } from "./HiveWorkerIntroductionSheet";
import { useActiveHiveWorkerBinding } from "./hooks/useActiveHiveWorkerBinding";
import {
  canShowHiveWorkerGoalForIntroduction,
  useActiveHiveWorkerIntroduction,
} from "./hooks/useActiveHiveWorkerIntroduction";
import { useHiveWorkerGoal } from "./hooks/useHiveWorkerGoal";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";

interface HiveMobileThreadControlsProps extends ActiveConversationActivity {
  workers: HiveWorkersState;
  primaryComposer: ReactNode;
  onSend: (sessionId: string, content: string) => Promise<void>;
  onStop: (sessionId: string) => void;
  composerHeight: number;
  introductionHeight: number;
  onComposerHeightChange: (height: number) => void;
  onIntroductionHeightChange: (height: number) => void;
  onGoalTrackerHeightChange: (height: number) => void;
}

/**
 * Adds Worker-owned controls to the one stable mobile transcript.
 *
 * The transcript remains owned by ActiveConversationSurface. This boundary
 * subscribes only to primitive session/tail state, then reuses the single Hive
 * roster to resolve the exact Worker DM and its cancellable detail reads.
 */
function HiveMobileThreadControlsComponent({
  workers,
  primaryComposer,
  sessionId,
  transcriptTailKey,
  isStreaming,
  isThinking,
  messageCount,
  onSend,
  onStop,
  composerHeight,
  introductionHeight,
  onComposerHeightChange,
  onIntroductionHeightChange,
  onGoalTrackerHeightChange,
}: HiveMobileThreadControlsProps) {
  const workerBinding = useActiveHiveWorkerBinding(
    workers,
    sessionId,
  );
  const activeWorker = workerBinding.worker;
  const introduction = useActiveHiveWorkerIntroduction({
    workers,
    worker: activeWorker,
    sessionId,
    transcriptTailKey,
    isStreaming,
  });
  const workerGoal = useHiveWorkerGoal({
    worker: activeWorker,
    sessionId: workerBinding.kind === "worker_dm" ? sessionId : null,
    transcriptTailKey,
    isStreaming,
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
  // The direct binding may resolve before the roster/detail projection. Keep
  // the composer read-only until the exact Introduction state is known.
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
  const showPrimaryHiveComposer = workerBinding.kind === "primary_hive" ||
    workerBinding.kind === "none";

  return (
    <>
      {workerBinding.kind === "worker_dm" && activeWorker &&
          activeWorker.status === "active" && introductionAllowsGoalTracker &&
          sessionId
        ? (
          <HiveWorkerGoalTracker
            key={`${activeWorker.id}:${sessionId}`}
            state={workerGoal}
            bottom={composerHeight + introductionHeight + 8}
            onHeightChange={onGoalTrackerHeightChange}
          />
        )
        : null}

      {workerBinding.error && !workerBinding.isResolving
        ? (
          <HiveMobileBindingError
            message={workerBinding.error}
            onRetry={workerBinding.retry}
            retryLabel="Retry Hive conversation binding"
            bottom={16}
          />
        )
        : null}

      {!workerBinding.error && introductionDetailError
        ? (
          <HiveMobileBindingError
            message={introductionDetailError}
            onRetry={introduction.refresh}
            retryLabel="Retry Worker details"
            bottom={composerHeight + 8}
          />
        )
        : null}

      <HiveWorkerIntroductionSheet
        state={introduction}
        bottom={composerHeight}
        onHeightChange={onIntroductionHeightChange}
      />

      {activeWorker && sessionId
        ? (
          <HiveWorkerComposer
            key={`${activeWorker.id}:${sessionId}`}
            worker={activeWorker}
            sessionId={sessionId}
            onSend={onSend}
            onStop={onStop}
            onHeightChange={onComposerHeightChange}
            isStreaming={isStreaming}
            disabled={workerDisablesComposer ||
              introductionWorkerDisablesComposer ||
              introductionDisablesComposer ||
              introductionIsResolving ||
              (messageCount === 0 && (isStreaming || isThinking))}
          />
        )
        : workerBinding.isResolving
        ? null
        : showPrimaryHiveComposer
        ? primaryComposer
        : null}
    </>
  );
}

export const HiveMobileThreadControls = memo(
  HiveMobileThreadControlsComponent,
);

function HiveMobileBindingError({
  message,
  onRetry,
  retryLabel,
  bottom,
}: {
  message: string;
  onRetry: () => void;
  retryLabel: string;
  bottom: number;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  return (
    <View
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      style={[
        styles.bindingError,
        {
          bottom,
          backgroundColor: t.background,
          borderColor: `${t.error}40`,
        },
      ]}
    >
      <Text style={[styles.bindingErrorText, { color: t.error }]}>
        {message}
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={retryLabel}
        onPress={onRetry}
        style={styles.bindingRetry}
      >
        <Text style={[styles.bindingRetryText, { color: t.userMessage }]}>
          Retry
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  bindingError: {
    position: "absolute",
    left: 16,
    right: 16,
    zIndex: 30,
    minHeight: 48,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  bindingErrorText: {
    flex: 1,
    minWidth: 0,
    fontSize: 12,
    lineHeight: 17,
  },
  bindingRetry: {
    minHeight: 36,
    justifyContent: "center",
    paddingHorizontal: 6,
  },
  bindingRetryText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "700",
  },
});
