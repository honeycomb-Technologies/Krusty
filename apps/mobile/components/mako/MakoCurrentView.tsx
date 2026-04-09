import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoSetCourseComposer } from "./MakoSetCourseComposer";
import {
  describeRun,
  formatProjectLabel,
  formatRelativeTime,
  formatTimestamp,
  getRunGroup,
  getRunNextWakeAt,
} from "./utils";
import type { MakoChatContext, MakoCurrentState } from "./types";
import type { ChatMessage } from "@krusty/api";

interface MakoCurrentViewProps {
  state: MakoCurrentState;
  workspaceDirectory?: string | null;
  model?: string | null;
  activeToolCallId?: string | null;
  chat: MakoChatContext;
  onSelectRun: (runId: string) => void;
  onOpenChat: () => void;
  onOpenRuns: () => void;
  onOpenDetails: () => void;
  onCourseSet: (runId: string) => Promise<void>;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
}

function SummaryCard({
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
    <View
      style={[
        styles.metricCell,
        {
          borderColor: t.border,
        },
      ]}
    >
      <Text style={[styles.metricLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.metricValue, { color: t.foreground }]}>{value}</Text>
      {hint ? (
        <Text style={[styles.metricHint, { color: t.mutedForeground }]}>{hint}</Text>
      ) : null}
    </View>
  );
}

function SectionTitle({ title }: { title: string }) {
  const { theme } = useThemeContext();
  return (
    <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}>
      {title}
    </Text>
  );
}

function FocusRow({
  tag,
  title,
  detail,
  primaryLabel,
  onPrimaryPress,
  secondaryLabel,
  onSecondaryPress,
  disabled = false,
}: {
  tag: string;
  title: string;
  detail: string;
  primaryLabel?: string;
  onPrimaryPress?: () => void;
  secondaryLabel?: string;
  onSecondaryPress?: () => void;
  disabled?: boolean;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.focusRow, { borderColor: t.border }]}>
      <View style={styles.focusCopy}>
        <Text style={[styles.focusTag, { color: t.mutedForeground }]}>{tag}</Text>
        <Text style={[styles.focusTitle, { color: t.foreground }]} numberOfLines={1}>
          {title}
        </Text>
        <Text style={[styles.focusDetail, { color: t.mutedForeground }]} numberOfLines={2}>
          {detail}
        </Text>
      </View>
      <View style={styles.focusActions}>
        {secondaryLabel && onSecondaryPress ? (
          <Pressable
            disabled={disabled}
            onPress={onSecondaryPress}
            style={styles.focusAction}
          >
            <Text
              style={[
                styles.focusActionText,
                { color: disabled ? `${t.mutedForeground}88` : t.mutedForeground },
              ]}
            >
              {secondaryLabel}
            </Text>
          </Pressable>
        ) : null}
        {primaryLabel && onPrimaryPress ? (
          <Pressable
            disabled={disabled}
            onPress={onPrimaryPress}
            style={styles.focusAction}
          >
            <Text
              style={[
                styles.focusActionText,
                { color: disabled ? `${t.userMessage}88` : t.userMessage },
              ]}
            >
              {primaryLabel}
            </Text>
          </Pressable>
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

export function MakoCurrentView({
  state,
  workspaceDirectory,
  model,
  activeToolCallId,
  chat,
  onSelectRun,
  onOpenChat,
  onOpenRuns,
  onOpenDetails,
  onCourseSet,
  onApproveTool,
}: MakoCurrentViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (state.isLoading && !state.current) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={t.userMessage} />
      </View>
    );
  }

  const runs = state.current?.runs ?? [];
  const waitingRuns = runs.filter((run) => getRunGroup(run) === "waiting");
  const activeRuns = runs.filter((run) => getRunGroup(run) === "active");
  const sleepingRuns = runs.filter((run) => getRunGroup(run) === "sleeping");
  const queuedRuns = runs.filter((run) => getRunGroup(run) === "queued");
  const approvals = state.current?.approvals ?? [];
  const status = state.current?.status;
  const focusApproval = approvals[0] ?? null;
  const focusRun = waitingRuns[0] ?? activeRuns[0] ?? null;
  const nextScheduledRun =
    [...queuedRuns, ...sleepingRuns]
      .sort((left, right) => {
        const leftValue = getRunNextWakeAt(left) ?? "9999";
        const rightValue = getRunNextWakeAt(right) ?? "9999";
        return leftValue.localeCompare(rightValue);
      })[0] ?? null;
  const recentMessages = chat.messages.slice(-3);

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={state.isRefreshing}
          onRefresh={() => {
            void state.refresh();
          }}
          tintColor={t.userMessage}
        />
      }
      showsVerticalScrollIndicator={false}
    >
      <Pressable
        onPress={onOpenDetails}
        style={[
          styles.metricsStrip,
          {
            borderTopColor: t.border,
            borderBottomColor: t.border,
          },
        ]}
      >
        <SummaryCard
          label="Running"
          value={String(status?.running_count ?? 0)}
          hint="awake now"
        />
        <SummaryCard
          label="Waiting"
          value={String(status?.waiting_count ?? 0)}
          hint="needs you"
        />
        <SummaryCard
          label="Next wake"
          value={
            status?.next_wake_at
              ? new Date(status.next_wake_at).toLocaleTimeString([], {
                  hour: "numeric",
                  minute: "2-digit",
                })
              : "None"
          }
          hint="scheduled"
        />
        <SummaryCard
          label="Health"
          value={status?.home_status ?? "idle"}
          hint="runtime"
        />
      </Pressable>

      <View style={styles.section}>
        <SectionTitle title="Focus" />
        <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
          {focusApproval ? (
            <FocusRow
              tag="Approval"
              title={focusApproval.tool_name}
              detail={`${formatProjectLabel(focusApproval.project_dir)} • requested ${formatRelativeTime(focusApproval.requested_at)}`}
              primaryLabel={activeToolCallId === null ? "Approve" : "Working..."}
              onPrimaryPress={() => {
                onApproveTool(focusApproval.session_id, focusApproval.tool_call_id);
              }}
              secondaryLabel="Open"
              onSecondaryPress={() => {
                onSelectRun(focusApproval.session_id);
              }}
              disabled={activeToolCallId !== null}
            />
          ) : null}

          {focusRun ? (
            <FocusRow
              tag="Run"
              title={focusRun.title || "Untitled run"}
              detail={describeRun(focusRun)}
              primaryLabel="Open"
              onPrimaryPress={() => {
                onSelectRun(focusRun.session_id);
              }}
            />
          ) : null}

          {nextScheduledRun ? (
            <FocusRow
              tag="Next"
              title={nextScheduledRun.title || "Untitled run"}
              detail={getRunNextWakeAt(nextScheduledRun)
                ? `scheduled ${formatTimestamp(getRunNextWakeAt(nextScheduledRun))}`
                : describeRun(nextScheduledRun)}
              primaryLabel="Open"
              onPrimaryPress={() => {
                onSelectRun(nextScheduledRun.session_id);
              }}
            />
          ) : null}

          {!focusApproval && !focusRun && !nextScheduledRun ? (
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              Nothing urgent right now.
            </Text>
          ) : null}
        </View>
      </View>

      <View style={styles.section}>
        <View style={styles.threadHeader}>
          <SectionTitle title="Thread" />
          <View style={styles.inlineActions}>
            <Pressable onPress={onOpenRuns} style={styles.inlineAction}>
              <Text style={[styles.inlineActionText, { color: t.mutedForeground }]}>
                Runs
              </Text>
            </Pressable>
            <Pressable onPress={onOpenChat} style={styles.inlineAction}>
              <Text style={[styles.inlineActionText, { color: t.userMessage }]}>
                Open chat
              </Text>
            </Pressable>
          </View>
        </View>
        <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
          {recentMessages.length ? (
            recentMessages.map((message) => (
              <View key={message.id} style={[styles.threadRow, { borderColor: t.border }]}>
                <Text style={[styles.threadRole, { color: t.mutedForeground }]}>
                  {message.role === "assistant" ? "Mako" : "You"}
                </Text>
                <Text
                  style={[styles.threadText, { color: t.foreground }]}
                  numberOfLines={2}
                >
                  {messagePreview(message)}
                </Text>
              </View>
            ))
          ) : (
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              No thread yet. Start with a direction, follow-up, or question.
            </Text>
          )}
        </View>
      </View>

      <MakoSetCourseComposer
        projectDir={workspaceDirectory}
        isSubmitting={state.isDispatching}
        onSubmit={async (task, options) => {
          const runId = await state.setCourse(task, {
            projectDir: workspaceDirectory ?? undefined,
            model: model ?? undefined,
            startAt: options?.startAt ?? undefined,
            priority: options?.priority ?? undefined,
          });
          if (runId) {
            await onCourseSet(runId);
          }
        }}
      />

      {state.error ? (
        <Text style={[styles.error, { color: t.error }]}>{state.error}</Text>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  content: {
    paddingBottom: 32,
    gap: 18,
  },
  loading: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  metricsStrip: {
    flexDirection: "row",
    paddingHorizontal: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  metricCell: {
    flex: 1,
    paddingVertical: 10,
    paddingHorizontal: 10,
    borderRightWidth: StyleSheet.hairlineWidth,
  },
  metricLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  metricValue: {
    marginTop: 4,
    fontSize: 15,
    fontWeight: "600",
  },
  metricHint: {
    marginTop: 2,
    fontSize: 11,
    lineHeight: 14,
  },
  section: {
    paddingHorizontal: 16,
  },
  sectionTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  sectionBody: {
    marginTop: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  focusRow: {
    flexDirection: "row",
    gap: 12,
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  focusCopy: {
    flex: 1,
    minWidth: 0,
  },
  focusTag: {
    fontSize: 11,
    fontWeight: "600",
  },
  focusTitle: {
    marginTop: 2,
    fontSize: 14,
    fontWeight: "600",
  },
  focusDetail: {
    marginTop: 4,
    fontSize: 12,
    lineHeight: 17,
  },
  focusActions: {
    alignItems: "flex-end",
    justifyContent: "center",
    gap: 8,
  },
  focusAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  focusActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  threadHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  inlineActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
  },
  inlineAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  inlineActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  threadRow: {
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 4,
  },
  threadRole: {
    fontSize: 11,
    fontWeight: "600",
  },
  threadText: {
    fontSize: 14,
    lineHeight: 19,
  },
  emptyText: {
    paddingVertical: 12,
    fontSize: 14,
    lineHeight: 19,
  },
  error: {
    paddingHorizontal: 16,
    fontSize: 13,
    lineHeight: 18,
  },
});
