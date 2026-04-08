import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoApprovalList } from "./MakoApprovalList";
import { MakoInsightCard } from "./MakoInsightCard";
import { MakoRunList } from "./MakoRunList";
import {
  describeRunDrift,
  formatTimestamp,
  getAttentionRuns,
  getQueueHeadRuns,
  getRunNextWakeAt,
  getRunPriority,
  getStaleRuns,
} from "./utils";
import type { MakoCurrentRunSummary, MakoCurrentState } from "./types";

interface MakoStatusViewProps {
  state: MakoCurrentState;
  activeToolCallId?: string | null;
  onSelectRun: (runId: string) => void;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
}

export function MakoStatusView({
  state,
  activeToolCallId,
  onSelectRun,
  onApproveTool,
  onDenyTool,
}: MakoStatusViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const status = state.current?.status;
  const runs = state.current?.runs ?? [];
  const approvals = state.current?.approvals ?? [];
  const attentionRuns = getAttentionRuns(runs);
  const staleRuns = getStaleRuns(runs);
  const queueHead = getQueueHeadRuns(runs);
  const cadence = summarizeCadence(runs);
  const queueHealth = summarizeQueueHealth(runs, approvals.length);
  const priorityProfile = summarizePriorityProfile(queueHead);
  const runtimeDrift = summarizeRuntimeDrift(staleRuns);
  const scheduledRuns = runs
    .filter(
      (run) =>
        run.runtime?.status === "sleeping" &&
        run.runtime.sleep_reason === "scheduled",
    )
    .sort((left, right) => {
      const leftValue = left.runtime?.next_wake_at ?? "";
      const rightValue = right.runtime?.next_wake_at ?? "";
      return leftValue.localeCompare(rightValue);
    });

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.wrap}
      showsVerticalScrollIndicator={false}
      refreshControl={
        <RefreshControl
          refreshing={state.isRefreshing}
          onRefresh={() => {
            void state.refresh();
          }}
          tintColor={t.userMessage}
        />
      }
    >
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Status keeps the control-plane truth compact: what is awake, what is queued, what is blocked, and when the next wake is expected.
      </Text>

      <View style={styles.grid}>
        <StatusCard label="Home state" value={status?.home_status ?? "idle"} />
        <StatusCard
          label="Approvals"
          value={String(status?.pending_approvals_count ?? 0)}
        />
        <StatusCard label="Running" value={String(status?.running_count ?? 0)} />
        <StatusCard
          label="Sleeping"
          value={String(status?.sleeping_count ?? 0)}
        />
        <StatusCard
          label="Scheduled"
          value={String(status?.scheduled_count ?? 0)}
        />
        <StatusCard
          label="High priority"
          value={String(status?.high_priority_count ?? 0)}
        />
        <StatusCard label="Drifting" value={String(staleRuns.length)} />
        <StatusCard label="Paused" value={String(status?.paused_count ?? 0)} />
        <StatusCard label="Failed" value={String(status?.failed_count ?? 0)} />
        <StatusCard label="Tick interval" value={cadence.tickIntervalLabel} />
        <StatusCard label="Tick budget" value={cadence.tickBudgetLabel} />
      </View>

      <GlassCard style={styles.card}>
        <Text style={[styles.cardLabel, { color: t.mutedForeground }]}>
          Next wake
        </Text>
        <Text style={[styles.cardValue, { color: t.foreground }]}>
          {formatTimestamp(status?.next_wake_at)}
        </Text>
      </GlassCard>

      <GlassCard style={styles.card}>
        <Text style={[styles.cardLabel, { color: t.mutedForeground }]}>
          Cadence
        </Text>
        <Text style={[styles.cardBody, { color: t.foreground }]}>
          {cadence.detail}
        </Text>
      </GlassCard>

      <View style={styles.insights}>
        <MakoInsightCard
          label="Queue health"
          value={queueHealth.value}
          detail={queueHealth.detail}
          tone={queueHealth.tone}
        />
        <MakoInsightCard
          label="Priority mix"
          value={priorityProfile.value}
          detail={priorityProfile.detail}
          tone={priorityProfile.tone}
        />
        <MakoInsightCard
          label="Runtime drift"
          value={runtimeDrift.value}
          detail={runtimeDrift.detail}
          tone={runtimeDrift.tone}
        />
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>
          Pending approvals
        </Text>
        <MakoApprovalList
          approvals={approvals}
          activeToolCallId={activeToolCallId}
          emptyLabel="No approvals are waiting."
          onSelectRun={onSelectRun}
          onApproveTool={onApproveTool}
          onDenyTool={onDenyTool}
        />
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>
          Needs attention
        </Text>
        <MakoRunList
          runs={attentionRuns}
          emptyLabel="Nothing is blocked or failed right now."
          onSelectRun={onSelectRun}
        />
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>
          Stalled or overdue
        </Text>
        <MakoRunList
          runs={staleRuns}
          emptyLabel="No runs look stalled right now."
          onSelectRun={onSelectRun}
          detailOverride={describeRunDrift}
        />
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>
          Upcoming wakes
        </Text>
        <MakoRunList
          runs={scheduledRuns}
          emptyLabel="No deferred wakes are queued."
          onSelectRun={onSelectRun}
        />
      </View>
    </ScrollView>
  );
}

function summarizeCadence(runs: MakoCurrentRunSummary[]) {
  if (!runs.length) {
    return {
      tickIntervalLabel: "30s",
      tickBudgetLabel: "1000",
      detail: "Default cadence applies until a project-specific Mako policy is present.",
    };
  }

  const profiles = new Map<string, number>();
  for (const run of runs) {
    const key = `${run.cadence.tick_interval_secs}:${run.cadence.max_ticks}`;
    profiles.set(key, (profiles.get(key) ?? 0) + 1);
  }

  if (profiles.size === 1) {
    const first = runs[0]?.cadence;
    return {
      tickIntervalLabel: `${first?.tick_interval_secs ?? 30}s`,
      tickBudgetLabel: String(first?.max_ticks ?? 1000),
      detail: "All visible runs share the same cadence policy.",
    };
  }

  return {
    tickIntervalLabel: "Mixed",
    tickBudgetLabel: `${profiles.size} profiles`,
    detail: `Visible runs currently span ${profiles.size} cadence profiles.`,
  };
}

function summarizeQueueHealth(
  runs: MakoCurrentRunSummary[],
  approvalCount: number,
) {
  const openRuns = getQueueHeadRuns(runs);
  const attentionRuns = getAttentionRuns(runs);
  const dueSoonCount = openRuns.filter((run) => {
    const wakeAt = getRunNextWakeAt(run);
    if (!wakeAt) {
      return false;
    }
    const diff = new Date(wakeAt).getTime() - Date.now();
    return diff > 0 && diff <= 60 * 60 * 1000;
  }).length;

  if (attentionRuns.length > 0 || approvalCount > 0) {
    return {
      value: "Attention",
      detail: `${attentionRuns.length} runs need intervention • ${approvalCount} approvals waiting • ${dueSoonCount} wake within 1h`,
      tone: "warning" as const,
    };
  }

  if (openRuns.length >= 6 || dueSoonCount >= 2) {
    return {
      value: "Busy",
      detail: `${openRuns.length} open runs are moving • ${dueSoonCount} wake within 1h`,
      tone: "accent" as const,
    };
  }

  return {
    value: "Calm",
    detail: `${openRuns.length} open runs • ${dueSoonCount} near-term wakes`,
    tone: "success" as const,
  };
}

function summarizePriorityProfile(runs: MakoCurrentRunSummary[]) {
  const counts = {
    high: runs.filter((run) => getRunPriority(run) === "high").length,
    normal: runs.filter((run) => getRunPriority(run) === "normal").length,
    low: runs.filter((run) => getRunPriority(run) === "low").length,
  };

  if (runs.length === 0) {
    return {
      value: "Quiet",
      detail: "No open runs are loaded into the queue right now.",
      tone: "default" as const,
    };
  }

  return {
    value: `${counts.high}H • ${counts.normal}N • ${counts.low}L`,
    detail:
      counts.high > 0
        ? "High-priority runs will stay ahead of the rest of the queue."
        : "No high-priority pressure is pushing on the queue right now.",
    tone: counts.high > 0 ? ("warning" as const) : ("default" as const),
  };
}

function summarizeRuntimeDrift(runs: MakoCurrentRunSummary[]) {
  const overdueWakeCount = runs.filter(
    (run) =>
      run.runtime?.status === "sleeping" &&
      run.runtime.sleep_reason === "scheduled" &&
      run.runtime.next_wake_at &&
      new Date(run.runtime.next_wake_at).getTime() < Date.now(),
  ).length;

  if (runs.length === 0) {
    return {
      value: "Healthy",
      detail: "No runs are currently drifting or overdue.",
      tone: "success" as const,
    };
  }

  return {
    value: `${runs.length} drifting`,
    detail: `${overdueWakeCount} overdue wakes • ${runs.length - overdueWakeCount} stale active or queued runs`,
    tone: overdueWakeCount > 0 ? ("warning" as const) : ("accent" as const),
  };
}

function StatusCard({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <GlassCard style={styles.statusCard}>
      <Text style={[styles.cardLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.cardValue, { color: t.foreground }]}>{value}</Text>
    </GlassCard>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  wrap: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 16,
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
  },
  statusCard: {
    width: "47%",
    marginBottom: 0,
  },
  card: {
    marginBottom: 0,
  },
  insights: {
    gap: 12,
  },
  cardLabel: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  cardValue: {
    marginTop: 10,
    fontSize: 22,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  cardBody: {
    marginTop: 10,
    fontSize: 14,
    lineHeight: 20,
  },
  section: {
    gap: 10,
  },
  sectionTitle: {
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
});
