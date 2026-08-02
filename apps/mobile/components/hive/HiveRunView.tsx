import { useEffect, useMemo, useState } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { RunDetailSkeleton } from "../ui/Skeleton";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { HiveCrewPicker } from "./HiveCrewPicker";
import { HivePriorityPicker } from "./HivePriorityPicker";
import { HiveSchedulePicker } from "./HiveSchedulePicker";
import { HiveWakeTimeline } from "./HiveWakeTimeline";
import { useHiveRun } from "./hooks/useHiveRun";
import { formatPriorityLabel } from "./priority";
import {
  formatScheduleInputValue,
  resolveScheduleSelection,
  type HiveSchedulePreset,
} from "./schedule";
import { HiveStatusBadge } from "./HiveStatusBadge";
import { HiveTopBar } from "./HiveTopBar";
import {
  describeRun,
  formatProjectLabel,
  formatRelativeTime,
  formatTimestamp,
  getRunDisplayStatus,
  getRunPriority,
  getRuntimeLabel,
} from "./utils";
import type { HiveChatContext, HiveCurrentRunSummary } from "./types";
import type { ChatMessage, HiveCrewRuntimeMember } from "@mitsuro/api";
import { useHiveSessionView } from "./hooks/useHiveSessionView";

interface HiveRunViewProps {
  runId: string;
  summary: HiveCurrentRunSummary | null;
  crewMembers: HiveCrewRuntimeMember[];
  chat: HiveChatContext;
  onBack: () => void;
  onDeleteRun: (id: string) => void;
}

function SummaryCell({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.summaryCell}>
      <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.summaryValue, { color: t.foreground }]} numberOfLines={1}>
        {value}
      </Text>
      {hint ? (
        <Text style={[styles.summaryHint, { color: t.mutedForeground }]} numberOfLines={1}>
          {hint}
        </Text>
      ) : null}
    </View>
  );
}

function SectionTitle({ title }: { title: string }) {
  const { theme } = useThemeContext();
  return <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}>{title}</Text>;
}

function FlatAction({
  label,
  color,
  disabled = false,
  onPress,
}: {
  label: string;
  color: string;
  disabled?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      disabled={disabled}
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onPress();
      }}
      style={styles.actionLink}
    >
      <Text style={[styles.actionLinkText, { color: disabled ? `${color}88` : color }]}>
        {label}
      </Text>
    </Pressable>
  );
}

function TaskRow({
  title,
  detail,
  status,
  timestamp,
}: {
  title: string;
  detail: string;
  status: string;
  timestamp?: string | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.listRow, { borderColor: t.border }]}>
      <View style={styles.listCopy}>
        <Text style={[styles.listTitle, { color: t.foreground }]} numberOfLines={1}>
          {title}
        </Text>
        <Text style={[styles.listDetail, { color: t.mutedForeground }]} numberOfLines={2}>
          {detail}
        </Text>
      </View>
      <View style={styles.listAside}>
        <HiveStatusBadge status={status} />
        {timestamp ? (
          <Text style={[styles.listMeta, { color: t.mutedForeground }]}>
            {formatTimestamp(timestamp)}
          </Text>
        ) : null}
      </View>
    </View>
  );
}

function messagePreview(message: ChatMessage): string {
  const content = message.content.trim();
  if (!content) {
    return message.role === "assistant" ? "Working..." : "No content";
  }
  return content.replace(/\s+/g, " ");
}

export function HiveRunView({
  runId,
  summary,
  crewMembers,
  chat,
  onBack,
  onDeleteRun,
}: HiveRunViewProps) {
  const sessionView = useHiveSessionView();
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [chatOpen, setChatOpen] = useState(false);
  const [schedulePreset, setSchedulePreset] = useState<HiveSchedulePreset>("30m");
  const [customSchedule, setCustomSchedule] = useState("");
  const [priority, setPriority] = useState(getRunPriority(summary ?? { runtime: null }));
  const [crewSlug, setCrewSlug] = useState(summary?.runtime?.crew_slug ?? null);
  const [isScheduling, setIsScheduling] = useState(false);
  const [isSavingPriority, setIsSavingPriority] = useState(false);
  const [isSavingCrew, setIsSavingCrew] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);
  const { status, wake, isLoading, refresh } = useHiveRun(runId, true);

  const displayStatus = getRunDisplayStatus(
    summary ?? {
      runtime: status?.runtime ?? null,
      agent_state: status?.agent_state ?? "idle",
    },
  );

  const taskStats = useMemo(() => {
    const tasks = status?.tasks ?? [];
    return {
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
  const runtimeCrewSlug = status?.runtime?.crew_slug ?? summary?.runtime?.crew_slug ?? null;
  const nextWakeAt = status?.runtime?.next_wake_at ?? summary?.runtime?.next_wake_at;
  const schedule = resolveScheduleSelection(schedulePreset, customSchedule);
  const resumeLabel = runtimeStatus === "sleeping" ? "Wake now" : "Resume";
  const showPause = runtimeStatus !== "sleeping" && runtimeStatus !== "paused";
  const showResume = runtimeStatus !== "running";
  const recentMessages = sessionView.messages.slice(-3);
  const artifactTasks = (status?.tasks ?? []).filter((task) => task.result);

  useEffect(() => {
    setPriority(runtimePriority);
  }, [runtimePriority]);

  useEffect(() => {
    setCrewSlug(runtimeCrewSlug);
  }, [runtimeCrewSlug]);

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
      await client.pauseHiveSession(runId);
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
      await client.resumeHiveSession(runId);
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
    if (!schedule.startAt) {
      setActionError("Choose a future wake time before rescheduling.");
      return;
    }
    setActionError(null);
    setIsScheduling(true);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.scheduleHiveSession(runId, schedule.startAt);
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
      await client.setHiveSessionPriority(runId, nextPriority);
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

  const handleCrewChange = async (nextCrewSlug: string | null) => {
    if (!client) {
      return;
    }
    setCrewSlug(nextCrewSlug);
    setActionError(null);
    setIsSavingCrew(true);
    try {
      await client.setHiveSessionCrew(runId, nextCrewSlug);
      await refresh();
    } catch (error) {
      setCrewSlug(runtimeCrewSlug);
      setActionError(
        error instanceof Error ? error.message : "Failed to update Hive Agent.",
      );
    } finally {
      setIsSavingCrew(false);
    }
  };

  if (chatOpen) {
    return (
      <View style={styles.container}>
        <HiveTopBar
          title={summary?.title || status?.title || "Run"}
          subtitle="Run chat"
          status={displayStatus}
          onBack={() => {
            setChatOpen(false);
          }}
        />

        <View style={styles.chatWrap}>
          <ChatTranscript
            messages={sessionView.messages}
            sessionId={sessionView.sessionId}
            isStreaming={sessionView.isStreaming}
            isThinking={sessionView.isThinking}
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
                <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
                  Talk directly to this run without leaving the watch.
                </Text>
              </View>
            }
            bottomPadding={composerReserveHeight}
            hideJumpToLatest={bottomControlsOpen}
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
            sessionType="hive"
            tokenCount={chat.tokenCount}
            onOverlayOpenChange={setBottomControlsOpen}
          />
        </View>
      </View>
    );
  }

  return (
    <View style={styles.container}>
      <HiveTopBar
        title={summary?.title || status?.title || "Run"}
        subtitle={formatProjectLabel(summary?.project_dir)}
        status={displayStatus}
        onBack={onBack}
      />

      {isLoading && !status ? (
        <RunDetailSkeleton />
      ) : (
        <ScrollView
          style={styles.scroll}
          contentContainerStyle={styles.content}
          showsVerticalScrollIndicator={false}
        >
          <View
            style={[
              styles.summaryStrip,
              {
                borderTopColor: t.border,
                borderBottomColor: t.border,
              },
            ]}
          >
            <SummaryCell
              label="State"
              value={getRuntimeLabel(displayStatus)}
              hint={formatPriorityLabel(runtimePriority)}
            />
            <SummaryCell
              label="Updated"
              value={formatRelativeTime(summary?.updated_at ?? status?.runtime?.updated_at)}
              hint={nextWakeAt ? `wake ${formatTimestamp(nextWakeAt)}` : "no wake set"}
            />
            <SummaryCell
              label="Tasks"
              value={String(taskStats.pending + taskStats.inProgress)}
              hint={`${taskStats.completed} done`}
            />
            <SummaryCell
              label="Agent"
              value={runtimeCrewSlug ?? "Hive"}
              hint={`${cadence.tick_interval_secs}s cadence`}
            />
          </View>

          <View style={styles.section}>
            <SectionTitle title="Overview" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <View style={styles.copyBlock}>
                <Text style={[styles.bodyText, { color: t.mutedForeground }]}>
                  {summary
                    ? describeRun(summary)
                    : "Runtime summary will populate as the run moves."}
                </Text>
                <View style={styles.metaRow}>
                  <HiveStatusBadge status={displayStatus} />
                  <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                    {formatTimestamp(status?.runtime?.updated_at)}
                  </Text>
                </View>
              </View>
              <View style={styles.actionRow}>
                <FlatAction
                  label="Open chat"
                  color={t.userMessage}
                  onPress={() => {
                    setChatOpen(true);
                  }}
                />
                {showPause ? (
                  <FlatAction label="Pause" color={t.mutedForeground} onPress={handlePause} />
                ) : null}
                {showResume ? (
                  <FlatAction label={resumeLabel} color={t.userMessage} onPress={handleResume} />
                ) : null}
                <FlatAction
                  label="Cancel"
                  color={t.error}
                  onPress={() => {
                    onDeleteRun(runId);
                  }}
                />
              </View>
              {actionError ? (
                <Text style={[styles.errorText, { color: t.error }]}>
                  {actionError}
                </Text>
              ) : null}
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Priority" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <Text style={[styles.bodyText, { color: t.mutedForeground }]}>
                {formatPriorityLabel(runtimePriority)}
              </Text>
              <View style={styles.controlBlock}>
                <HivePriorityPicker
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
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Agent" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <Text style={[styles.bodyText, { color: t.mutedForeground }]}>
                Assign this run to Hive or a specific Hive Agent. The selected identity shapes the run&apos;s working presence and context layers.
              </Text>
              <View style={styles.controlBlock}>
                <HiveCrewPicker
                  members={crewMembers}
                  selectedSlug={crewSlug}
                  isSaving={isSavingCrew}
                  onSelect={(nextCrewSlug) => {
                    void handleCrewChange(nextCrewSlug);
                  }}
                />
              </View>
              {isSavingCrew ? (
                <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                  Saving agent...
                </Text>
              ) : null}
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Schedule" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <Text style={[styles.bodyText, { color: t.mutedForeground }]}>
                {nextWakeAt
                  ? `Next wake is ${formatTimestamp(nextWakeAt)}.`
                  : "No future wake is set right now."}
              </Text>
              <View style={styles.controlBlock}>
                <HiveSchedulePicker
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
                <FlatAction
                  label={isScheduling ? "Scheduling..." : "Reschedule"}
                  color={t.userMessage}
                  disabled={
                    isScheduling ||
                    (schedulePreset === "custom" && schedule.error !== null)
                  }
                  onPress={() => {
                    void handleSchedule();
                  }}
                />
              </View>
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Wake" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <HiveWakeTimeline wake={wake} />
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Tasks" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              {(status?.tasks ?? []).length === 0 ? (
                <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
                  No tasks yet.
                </Text>
              ) : (
                (status?.tasks ?? []).map((task) => (
                  <TaskRow
                    key={task.id}
                    title={task.subject}
                    detail={task.description || task.result || "No details yet."}
                    status={task.status}
                    timestamp={task.updated_at}
                  />
                ))
              )}
            </View>
          </View>

          <View style={styles.section}>
            <View style={styles.sectionHeader}>
              <SectionTitle title="Recent thread" />
              <FlatAction
                label="Open chat"
                color={t.userMessage}
                onPress={() => {
                  setChatOpen(true);
                }}
              />
            </View>
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              {recentMessages.length === 0 ? (
                <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
                  No chat yet.
                </Text>
              ) : (
                recentMessages.map((message) => (
                  <View key={message.id} style={[styles.listRow, { borderColor: t.border }]}>
                    <View style={styles.listCopy}>
                      <Text style={[styles.listMeta, { color: t.mutedForeground }]}>
                        {message.role === "assistant" ? "Hive" : "You"}
                      </Text>
                      <Text
                        style={[styles.listDetail, { color: t.foreground }]}
                        numberOfLines={2}
                      >
                        {messagePreview(message)}
                      </Text>
                    </View>
                  </View>
                ))
              )}
            </View>
          </View>

          <View style={styles.section}>
            <SectionTitle title="Artifacts" />
            <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
              <TaskRow
                title="Project"
                detail={formatProjectLabel(summary?.project_dir)}
                status="idle"
              />
              {artifactTasks.map((task) => (
                <TaskRow
                  key={`artifact-${task.id}`}
                  title={task.subject}
                  detail={task.result || "No result yet."}
                  status={task.status}
                  timestamp={task.updated_at}
                />
              ))}
            </View>
          </View>
        </ScrollView>
      )}
    </View>
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
    paddingBottom: 28,
    gap: 16,
  },
  loading: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  summaryStrip: {
    flexDirection: "row",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
  },
  summaryCell: {
    flex: 1,
    paddingVertical: 10,
    paddingHorizontal: 10,
  },
  summaryLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  summaryValue: {
    marginTop: 4,
    fontSize: 14,
    fontWeight: "600",
  },
  summaryHint: {
    marginTop: 2,
    fontSize: 11,
    lineHeight: 14,
  },
  section: {
    paddingHorizontal: 16,
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  sectionTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  sectionBody: {
    marginTop: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  copyBlock: {
    paddingVertical: 12,
    gap: 8,
  },
  bodyText: {
    fontSize: 13,
    lineHeight: 18,
  },
  metaRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  metaText: {
    fontSize: 12,
    lineHeight: 16,
  },
  controlBlock: {
    paddingTop: 12,
  },
  actionRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 14,
    paddingTop: 10,
  },
  actionLink: {
    minHeight: 24,
    justifyContent: "center",
  },
  actionLinkText: {
    fontSize: 12,
    fontWeight: "600",
  },
  errorText: {
    paddingTop: 10,
    fontSize: 13,
    lineHeight: 18,
  },
  listRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  listCopy: {
    flex: 1,
    minWidth: 0,
  },
  listTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  listDetail: {
    marginTop: 4,
    fontSize: 13,
    lineHeight: 18,
  },
  listAside: {
    alignItems: "flex-end",
    gap: 6,
  },
  listMeta: {
    fontSize: 11,
    lineHeight: 15,
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
    fontSize: 18,
    fontWeight: "600",
  },
  emptyBody: {
    paddingVertical: 12,
    fontSize: 13,
    lineHeight: 18,
  },
});
