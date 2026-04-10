import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoThreadSurface } from "./MakoThreadSurface";
import {
  describeRun,
  formatProjectLabel,
  formatRelativeTime,
  getRunGroup,
  getRunNextWakeAt,
} from "./utils";
import type {
  MakoChatContext,
  MakoCurrentState,
  MakoHomeState,
} from "./types";

interface MakoCurrentViewProps {
  state: MakoCurrentState;
  homeState: MakoHomeState;
  chat: MakoChatContext;
  onSelectRun: (runId: string) => void;
  onOpenRuns: () => void;
  onOpenDetails: () => void;
  onOpenSchedule: () => void;
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
    <View style={styles.metricCell}>
      <Text style={[styles.metricLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.metricValue, { color: t.foreground }]}>{value}</Text>
      {hint ? (
        <Text style={[styles.metricHint, { color: t.mutedForeground }]}>{hint}</Text>
      ) : null}
    </View>
  );
}

function SectionTitle({
  title,
  actionLabel,
  onAction,
}: {
  title: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.sectionHeader}>
      <Text style={[styles.sectionTitle, { color: t.foreground }]}>{title}</Text>
      {actionLabel && onAction ? (
        <Pressable onPress={onAction} style={styles.headerAction}>
          <Text style={[styles.headerActionText, { color: t.userMessage }]}>
            {actionLabel}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

function FocusRow({
  tag,
  title,
  detail,
  primaryLabel,
  onPrimaryPress,
}: {
  tag: string;
  title: string;
  detail: string;
  primaryLabel: string;
  onPrimaryPress: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.row, { borderColor: t.border }]}>
      <View style={styles.rowCopy}>
        <Text style={[styles.rowTag, { color: t.mutedForeground }]}>{tag}</Text>
        <Text style={[styles.rowTitle, { color: t.foreground }]} numberOfLines={1}>
          {title}
        </Text>
        <Text style={[styles.rowDetail, { color: t.mutedForeground }]} numberOfLines={2}>
          {detail}
        </Text>
      </View>
      <Pressable onPress={onPrimaryPress} style={styles.rowAction}>
        <Text style={[styles.rowActionText, { color: t.userMessage }]}>
          {primaryLabel}
        </Text>
      </Pressable>
    </View>
  );
}

function PresenceRow({
  label,
  value,
  onPress,
}: {
  label: string;
  value: string;
  onPress?: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <Pressable
      disabled={!onPress}
      onPress={onPress}
      style={[styles.presenceRow, { borderColor: t.border }]}
    >
      <Text style={[styles.presenceLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.presenceValue, { color: t.foreground }]} numberOfLines={2}>
        {value}
      </Text>
    </Pressable>
  );
}

function previewText(value?: string | null, fallback = "Not configured yet.") {
  const trimmed = value?.trim();
  if (!trimmed) {
    return fallback;
  }
  return trimmed;
}

export function MakoCurrentView({
  state,
  homeState,
  chat,
  onSelectRun,
  onOpenRuns,
  onOpenDetails,
  onOpenSchedule,
}: MakoCurrentViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (state.isLoading && !state.current && homeState.isLoading && !homeState.home) {
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

  const home = homeState.home;
  const crew = homeState.crew?.members ?? [];
  const runningCrew = crew.filter((member) => member.status === "running").length;
  const waitingCrew = crew.filter((member) => member.status === "waiting").length;
  const degradedCrew = crew.filter((member) => member.status === "degraded").length;
  const needsBootstrap =
    !homeState.isLoading &&
    !home?.soul &&
    !home?.identity &&
    !home?.heartbeat &&
    !home?.channels &&
    (home?.crew_count ?? 0) === 0;

  const topError = state.error ?? homeState.error;

  return (
    <View style={styles.container}>
      <View style={[styles.headerBlock, { borderBottomColor: t.border }]}>
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
          <SummaryCell
            label="Running"
            value={String(status?.running_count ?? 0)}
            hint="awake now"
          />
          <SummaryCell
            label="Waiting"
            value={String(status?.waiting_count ?? 0)}
            hint="needs you"
          />
          <SummaryCell
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
          <SummaryCell
            label="State"
            value={status?.home_status ?? "idle"}
            hint="details"
          />
        </Pressable>

        <View style={styles.quickActions}>
          <Pressable onPress={onOpenRuns} style={styles.quickAction}>
            <Text style={[styles.quickActionText, { color: t.mutedForeground }]}>
              Runs
            </Text>
          </Pressable>
          <Pressable onPress={onOpenSchedule} style={styles.quickAction}>
            <Text style={[styles.quickActionText, { color: t.mutedForeground }]}>
              Schedule
            </Text>
          </Pressable>
          <Pressable onPress={onOpenDetails} style={styles.quickAction}>
            <Text style={[styles.quickActionText, { color: t.userMessage }]}>
              Details
            </Text>
          </Pressable>
        </View>

        {topError ? (
          <Text style={[styles.errorText, { color: t.error }]}>{topError}</Text>
        ) : null}

        <View style={styles.section}>
          <SectionTitle title="Focus" />
          <View style={styles.rows}>
            {focusApproval ? (
              <FocusRow
                tag="Approval"
                title={focusApproval.tool_name}
                detail={`${formatProjectLabel(focusApproval.project_dir)} • requested ${formatRelativeTime(focusApproval.requested_at)}`}
                primaryLabel="Open"
                onPrimaryPress={() => {
                  onSelectRun(focusApproval.session_id);
                }}
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
                tag="Next wake"
                title={nextScheduledRun.title || "Untitled run"}
                detail={getRunNextWakeAt(nextScheduledRun)
                  ? `scheduled ${new Date(getRunNextWakeAt(nextScheduledRun) as string).toLocaleString([], {
                      month: "short",
                      day: "numeric",
                      hour: "numeric",
                      minute: "2-digit",
                    })}`
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
          <SectionTitle
            title="Presence"
            actionLabel={needsBootstrap ? "Initialize" : "Manage"}
            onAction={() => {
              if (needsBootstrap) {
                void homeState.bootstrap();
              } else {
                onOpenDetails();
              }
            }}
          />
          <View style={styles.rows}>
            {needsBootstrap ? (
              <View style={[styles.row, { borderColor: t.border }]}>
                <View style={styles.rowCopy}>
                  <Text style={[styles.rowTitle, { color: t.foreground }]}>
                    Mako home is not initialized yet.
                  </Text>
                  <Text style={[styles.rowDetail, { color: t.mutedForeground }]}>
                    Create the soul, identity, heartbeat, memory, channels, and default crew files.
                  </Text>
                </View>
                <Pressable
                  onPress={() => {
                    void homeState.bootstrap();
                  }}
                  style={styles.rowAction}
                  disabled={homeState.isBootstrapping}
                >
                  <Text style={[styles.rowActionText, { color: t.userMessage }]}>
                    {homeState.isBootstrapping ? "Initializing..." : "Initialize"}
                  </Text>
                </Pressable>
              </View>
            ) : (
              <>
                <PresenceRow
                  label="Soul"
                  value={previewText(home?.soul?.preview)}
                  onPress={onOpenDetails}
                />
                <PresenceRow
                  label="Heartbeat"
                  value={previewText(home?.heartbeat?.preview)}
                  onPress={onOpenDetails}
                />
                <PresenceRow
                  label="Crew"
                  value={
                    crew.length
                      ? `${crew.length} crew • ${runningCrew} active • ${waitingCrew} waiting${degradedCrew > 0 ? ` • ${degradedCrew} degraded` : ""}`
                      : "No crew configured yet."
                  }
                  onPress={onOpenDetails}
                />
              </>
            )}
          </View>
        </View>
      </View>

      <View style={styles.threadWrap}>
        <MakoThreadSurface
          chat={chat}
          emptyTitle="This is the main Mako thread"
          emptyBody="Use this conversation to steer Mako, ask what it is doing, approve work, or redirect the next move."
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  headerBlock: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingBottom: 8,
  },
  metricsStrip: {
    flexDirection: "row",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    paddingVertical: 10,
  },
  metricCell: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  metricLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  metricValue: {
    fontSize: 15,
    fontWeight: "600",
  },
  metricHint: {
    fontSize: 11,
    lineHeight: 14,
  },
  quickActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
    paddingHorizontal: 16,
    paddingTop: 8,
  },
  quickAction: {
    paddingVertical: 2,
  },
  quickActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  errorText: {
    paddingHorizontal: 16,
    paddingTop: 8,
    fontSize: 12,
    lineHeight: 18,
  },
  section: {
    paddingHorizontal: 16,
    paddingTop: 12,
    gap: 8,
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  headerAction: {
    paddingVertical: 2,
  },
  headerActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  rows: {
    gap: 6,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 4,
  },
  rowCopy: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  rowTag: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
  },
  rowTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  rowDetail: {
    fontSize: 12,
    lineHeight: 17,
  },
  rowAction: {
    paddingVertical: 4,
  },
  rowActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  emptyText: {
    fontSize: 12,
    lineHeight: 18,
    paddingVertical: 6,
  },
  presenceRow: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 4,
    gap: 4,
  },
  presenceLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
  },
  presenceValue: {
    fontSize: 12,
    lineHeight: 18,
  },
  threadWrap: {
    flex: 1,
    minHeight: 0,
  },
});
