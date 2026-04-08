import { useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { GlassCard } from "../ui/GlassCard";
import { MakoPriorityPicker } from "./MakoPriorityPicker";
import { MakoSchedulePicker } from "./MakoSchedulePicker";
import { MakoWakeTimeline } from "./MakoWakeTimeline";
import { useMakoRun } from "./hooks/useMakoRun";
import { formatPriorityLabel } from "./priority";
import {
  formatScheduleInputValue,
  resolveScheduleSelection,
  type MakoSchedulePreset,
} from "./schedule";
import { MakoStatusBadge } from "./MakoStatusBadge";
import { MakoTopBar } from "./MakoTopBar";
import { MakoTopNav } from "./MakoTopNav";
import {
  describeRun,
  formatProjectLabel,
  formatRelativeTime,
  formatTimestamp,
  getRunPriority,
  getRunDisplayStatus,
  getRuntimeLabel,
} from "./utils";
import type { MakoChatContext, MakoCurrentRunSummary } from "./types";

interface MakoRunViewProps {
  runId: string;
  summary: MakoCurrentRunSummary | null;
  chat: MakoChatContext;
  onBack: () => void;
  onDeleteRun: (id: string) => void;
}

export function MakoRunView({
  runId,
  summary,
  chat,
  onBack,
  onDeleteRun,
}: MakoRunViewProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [runSection, setRunSection] = useState<
    "overview" | "wake" | "tasks" | "chat" | "artifacts"
  >("overview");
  const [schedulePreset, setSchedulePreset] = useState<MakoSchedulePreset>("30m");
  const [customSchedule, setCustomSchedule] = useState("");
  const [priority, setPriority] = useState(getRunPriority(summary ?? { runtime: null }));
  const [isScheduling, setIsScheduling] = useState(false);
  const [isSavingPriority, setIsSavingPriority] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);
  const { status, wake, isLoading, refresh } = useMakoRun(runId, true);

  const displayStatus = getRunDisplayStatus(summary ?? {
    runtime: status?.runtime ?? null,
    agent_state: status?.agent_state ?? "idle",
  });

  const taskStats = useMemo(() => {
    const tasks = status?.tasks ?? [];
    return {
      total: tasks.length,
      inProgress: tasks.filter((task) => task.status === "in_progress").length,
      pending: tasks.filter((task) => task.status === "pending").length,
      completed: tasks.filter((task) => task.status === "completed").length,
      failed: tasks.filter((task) => task.status === "failed").length,
    };
  }, [status?.tasks]);
  const cadence = status?.cadence ?? summary?.cadence ?? {
    tick_interval_secs: 30,
    max_ticks: 1000,
  };
  const runtimeStatus = status?.runtime?.status ?? null;
  const runtimePriority = status?.runtime?.priority ?? summary?.runtime?.priority ?? "normal";
  const nextWakeAt = status?.runtime?.next_wake_at ?? summary?.runtime?.next_wake_at;
  const schedule = resolveScheduleSelection(schedulePreset, customSchedule);
  const resumeLabel = runtimeStatus === "sleeping" ? "Wake now" : "Resume";
  const showPause = runtimeStatus !== "sleeping" && runtimeStatus !== "paused";
  const showResume = runtimeStatus !== "running";

  useEffect(() => {
    setPriority(runtimePriority);
  }, [runtimePriority]);

  useEffect(() => {
    setCustomSchedule(formatScheduleInputValue(nextWakeAt));
  }, [nextWakeAt]);

  const handlePause = async () => {
    if (!client) {
      return;
    }
    setActionError(null);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.pauseMakoSession(runId);
      await refresh();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to pause this run.",
      );
    }
  };

  const handleResume = async () => {
    if (!client) {
      return;
    }
    setActionError(null);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.resumeMakoSession(runId);
      await refresh();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to wake this run.",
      );
    }
  };

  const handleSchedule = async () => {
    if (!client) {
      return;
    }
    if (schedule.error) {
      setActionError(schedule.error);
      return;
    }
    const startAt = schedule.startAt;
    if (!startAt) {
      setActionError("Choose a future wake time before rescheduling.");
      return;
    }
    setActionError(null);
    setIsScheduling(true);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.scheduleMakoSession(runId, startAt);
      await refresh();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to reschedule this run.",
      );
    } finally {
      setIsScheduling(false);
    }
  };

  const handlePriorityChange = async (nextPriority: typeof priority) => {
    if (!client) {
      return;
    }
    setPriority(nextPriority);
    setActionError(null);
    setIsSavingPriority(true);
    try {
      await client.setMakoSessionPriority(runId, nextPriority);
      await refresh();
    } catch (error) {
      setPriority(runtimePriority);
      setActionError(
        error instanceof Error ? error.message : "Failed to update priority.",
      );
    } finally {
      setIsSavingPriority(false);
    }
  };

  return (
    <View style={styles.container}>
      <MakoTopBar
        title={summary?.title || status?.title || "Run"}
        subtitle={formatProjectLabel(summary?.project_dir)}
        status={displayStatus}
        onBack={onBack}
      />

      <MakoTopNav
        items={[
          { id: "overview", label: "Overview" },
          { id: "wake", label: "Wake" },
          { id: "tasks", label: "Tasks" },
          { id: "chat", label: "Chat" },
          { id: "artifacts", label: "Artifacts" },
        ]}
        active={runSection}
        onSelect={setRunSection}
      />

      {isLoading && !status ? (
        <View style={styles.loading}>
          <ActivityIndicator color={t.userMessage} />
        </View>
      ) : null}

      {!isLoading || status ? (
        <>
          {runSection === "overview" ? (
            <ScrollView
              style={styles.scroll}
              contentContainerStyle={styles.content}
              showsVerticalScrollIndicator={false}
            >
              <View style={styles.metricsRow}>
                <OverviewCard label="State" value={getRuntimeLabel(displayStatus)} />
                <OverviewCard
                  label="Updated"
                  value={formatRelativeTime(summary?.updated_at ?? status?.runtime?.updated_at)}
                />
              </View>

              <View style={styles.metricsRow}>
                <OverviewCard label="Open tasks" value={String(taskStats.pending + taskStats.inProgress)} />
                <OverviewCard label="Completed" value={String(taskStats.completed)} />
              </View>

              <View style={styles.metricsRow}>
                <OverviewCard label="Tick interval" value={`${cadence.tick_interval_secs}s`} />
                <OverviewCard label="Tick budget" value={String(cadence.max_ticks)} />
              </View>

              <GlassCard style={styles.card}>
                <Text style={[styles.cardTitle, { color: t.foreground }]}>
                  Run state
                </Text>
                <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                  {summary ? describeRun(summary) : "Runtime summary will populate as the run moves."}
                </Text>
                <View style={styles.rowMeta}>
                  <MakoStatusBadge status={displayStatus} />
                  <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                    {formatTimestamp(status?.runtime?.updated_at)}
                  </Text>
                </View>
              </GlassCard>

              <GlassCard style={styles.card}>
                <Text style={[styles.cardTitle, { color: t.foreground }]}>
                  Run priority
                </Text>
                <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                  {formatPriorityLabel(runtimePriority)}
                </Text>
                <View style={styles.scheduleWrap}>
                  <MakoPriorityPicker
                    value={priority}
                    onChange={(nextPriority) => {
                      void handlePriorityChange(nextPriority);
                    }}
                  />
                </View>
                {isSavingPriority ? (
                  <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                    Saving priority...
                  </Text>
                ) : null}
              </GlassCard>

              <GlassCard style={styles.card}>
                <Text style={[styles.cardTitle, { color: t.foreground }]}>
                  Course timing
                </Text>
                <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                  {nextWakeAt
                    ? `Next wake is ${formatTimestamp(nextWakeAt)}.`
                    : "No future wake is set right now."}
                </Text>
                <View style={styles.scheduleWrap}>
                  <MakoSchedulePicker
                    value={schedulePreset}
                    onChange={setSchedulePreset}
                    includeImmediate={false}
                    subject="run"
                    customValue={customSchedule}
                    onCustomValueChange={setCustomSchedule}
                    customError={schedulePreset === "custom" ? schedule.error : null}
                  />
                </View>
                <View style={styles.actionRow}>
                  <ActionButton
                    label={isScheduling ? "Scheduling..." : "Reschedule"}
                    tone="primary"
                    disabled={
                      isScheduling ||
                      (schedulePreset === "custom" && schedule.error !== null)
                    }
                    onPress={() => {
                      void handleSchedule();
                    }}
                  />
                </View>
              </GlassCard>

              <View style={styles.actionRow}>
                {showPause ? (
                  <ActionButton
                    label="Pause"
                    tone="neutral"
                    onPress={() => {
                      void handlePause();
                    }}
                  />
                ) : null}
                {showResume ? (
                  <ActionButton
                    label={resumeLabel}
                    tone="primary"
                    onPress={() => {
                      void handleResume();
                    }}
                  />
                ) : null}
                <ActionButton
                  label="Cancel"
                  tone="danger"
                  onPress={() => onDeleteRun(runId)}
                />
              </View>

              {actionError ? (
                <Text style={[styles.errorText, { color: t.error }]}>
                  {actionError}
                </Text>
              ) : null}
            </ScrollView>
          ) : null}

          {runSection === "wake" ? (
            <ScrollView
              style={styles.scroll}
              contentContainerStyle={styles.content}
              showsVerticalScrollIndicator={false}
            >
              <MakoWakeTimeline wake={wake} />
            </ScrollView>
          ) : null}

          {runSection === "tasks" ? (
            <ScrollView
              style={styles.scroll}
              contentContainerStyle={styles.content}
              showsVerticalScrollIndicator={false}
            >
              {(status?.tasks ?? []).map((task) => (
                <GlassCard key={task.id} style={styles.card}>
                  <View style={styles.rowMeta}>
                    <Text style={[styles.cardTitle, { color: t.foreground, flex: 1 }]}>
                      {task.subject}
                    </Text>
                    <MakoStatusBadge status={task.status} />
                  </View>
                  <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                    {task.description || task.result || "No details yet."}
                  </Text>
                  <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                    {formatTimestamp(task.updated_at)}
                  </Text>
                </GlassCard>
              ))}
            </ScrollView>
          ) : null}

          {runSection === "chat" ? (
            <View style={styles.chatWrap}>
              <ChatTranscript
                messages={chat.messages}
                sessionId={chat.sessionId}
                isStreaming={chat.isStreaming}
                isThinking={chat.isThinking}
                activeToolCallId={chat.activeToolCallId}
                onApproveTool={chat.onApproveTool}
                onDenyTool={chat.onDenyTool}
                onSubmitToolResult={chat.onSubmitToolResult}
                onPlanConfirm={chat.onPlanConfirm}
                emptyState={
                  <View style={styles.emptyChat}>
                    <Text style={[styles.emptyTitle, { color: t.foreground }]}>
                      No chat yet
                    </Text>
                    <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                      Talk directly to this run without leaving the watch.
                    </Text>
                  </View>
                }
                bottomPadding={composerReserveHeight}
                showPlanTracker={false}
              />

              <ChatBar
                onSend={chat.onSend}
                onStop={chat.onStop}
                onHeightChange={setComposerReserveHeight}
                isStreaming={chat.isStreaming}
                disabled={!chat.sessionId}
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
                models={chat.models}
                sessionType="mako"
                researchEnabled={chat.researchEnabled}
                onResearchToggle={chat.onResearchToggle}
                tokenCount={chat.tokenCount}
              />
            </View>
          ) : null}

          {runSection === "artifacts" ? (
            <ScrollView
              style={styles.scroll}
              contentContainerStyle={styles.content}
              showsVerticalScrollIndicator={false}
            >
              <GlassCard style={styles.card}>
                <Text style={[styles.cardTitle, { color: t.foreground }]}>
                  Project
                </Text>
                <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                  {formatProjectLabel(summary?.project_dir)}
                </Text>
              </GlassCard>

              {(status?.tasks ?? [])
                .filter((task) => task.result)
                .map((task) => (
                  <GlassCard key={`artifact-${task.id}`} style={styles.card}>
                    <Text style={[styles.cardTitle, { color: t.foreground }]}>
                      {task.subject}
                    </Text>
                    <Text style={[styles.cardBody, { color: t.mutedForeground }]}>
                      {task.result}
                    </Text>
                  </GlassCard>
                ))}
            </ScrollView>
          ) : null}
        </>
      ) : null}
    </View>
  );
}

function OverviewCard({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  return (
    <GlassCard style={styles.metricCard}>
      <Text style={[styles.metricLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.metricValue, { color: t.foreground }]}>{value}</Text>
    </GlassCard>
  );
}

function ActionButton({
  label,
  tone,
  disabled = false,
  onPress,
}: {
  label: string;
  tone: "primary" | "neutral" | "danger";
  disabled?: boolean;
  onPress: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const palette =
    tone === "primary"
      ? { backgroundColor: t.userMessage, color: "#ffffff" }
      : tone === "danger"
        ? { backgroundColor: `${t.error}18`, color: t.error }
        : { backgroundColor: t.glass.backgroundElevated, color: t.foreground };

  return (
    <Pressable
      disabled={disabled}
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onPress();
      }}
      style={[
        styles.actionButton,
        {
          backgroundColor: palette.backgroundColor,
          opacity: disabled ? 0.6 : 1,
        },
      ]}
    >
      <Text style={[styles.actionLabel, { color: palette.color }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  scroll: {
    flex: 1,
  },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 12,
  },
  loading: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  metricsRow: {
    flexDirection: "row",
    gap: 12,
  },
  metricCard: {
    flex: 1,
    marginBottom: 0,
  },
  metricLabel: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  metricValue: {
    marginTop: 10,
    fontSize: 22,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  card: {
    marginBottom: 0,
  },
  cardTitle: {
    fontSize: 16,
    fontWeight: "700",
  },
  cardBody: {
    marginTop: 10,
    fontSize: 13,
    lineHeight: 18,
  },
  rowMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    marginTop: 14,
  },
  scheduleWrap: {
    marginTop: 14,
  },
  metaText: {
    fontSize: 12,
    fontWeight: "500",
  },
  actionRow: {
    flexDirection: "row",
    gap: 10,
  },
  errorText: {
    fontSize: 13,
    lineHeight: 18,
  },
  actionButton: {
    flex: 1,
    borderRadius: 16,
    paddingVertical: 12,
    alignItems: "center",
  },
  actionLabel: {
    fontSize: 13,
    fontWeight: "700",
  },
  chatWrap: {
    flex: 1,
  },
  emptyChat: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
    paddingHorizontal: 32,
    gap: 10,
  },
  emptyTitle: {
    fontSize: 20,
    fontWeight: "700",
  },
});
