import {
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoApprovalList } from "./MakoApprovalList";
import { MakoPresenceDetails } from "./MakoPresenceDetails";
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
import type {
  MakoCurrentRunSummary,
  MakoCurrentState,
  MakoHomeState,
} from "./types";
import type {
  MakoDaemonSummary,
  MakoDiagnosticsSummary,
  MakoHealthState,
  MakoKnowledgeHealthSummary,
} from "@krusty/api";

interface MakoStatusViewProps {
  state: MakoCurrentState;
  homeState: MakoHomeState;
  activeToolCallId?: string | null;
  onSelectRun: (runId: string) => void;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
}

export function MakoStatusView({
  state,
  homeState,
  activeToolCallId,
  onSelectRun,
  onApproveTool,
  onDenyTool,
}: MakoStatusViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const status = state.current?.status;
  const diagnostics = state.current?.diagnostics;
  const daemon = diagnostics?.daemon;
  const runs = state.current?.runs ?? [];
  const approvals = state.current?.approvals ?? [];
  const attentionRuns = getAttentionRuns(runs);
  const staleRuns = getStaleRuns(runs);
  const queueHead = getQueueHeadRuns(runs);
  const cadence = summarizeCadence(runs);
  const queueHealth = summarizeQueueHealth(runs, approvals.length, diagnostics);
  const priorityProfile = summarizePriorityProfile(queueHead);
  const runtimeDrift = summarizeRuntimeDrift(staleRuns, diagnostics);
  const knowledgeHealth = summarizeKnowledgeHealth(diagnostics?.knowledge);
  const daemonHealth = summarizeDaemonHealth(daemon, diagnostics?.health_state);
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
        Details keeps the control-plane truth compact: what is awake, what is blocked, how healthy the daemon is, and what may need intervention next.
      </Text>

      <MakoPresenceDetails state={homeState} />

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
          label="Health"
          value={formatHealthState(diagnostics?.health_state)}
        />
        <SummaryCell label="Running" value={String(status?.running_count ?? 0)} />
        <SummaryCell label="Waiting" value={String(status?.pending_approvals_count ?? 0)} />
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
        />
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>Signals</Text>
        <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
          <SignalRow
            label="Queue health"
            value={queueHealth.value}
            detail={queueHealth.detail}
            tone={queueHealth.tone}
          />
          <SignalRow
            label="Priority mix"
            value={priorityProfile.value}
            detail={priorityProfile.detail}
            tone={priorityProfile.tone}
          />
          <SignalRow
            label="Runtime drift"
            value={runtimeDrift.value}
            detail={runtimeDrift.detail}
            tone={runtimeDrift.tone}
          />
          <SignalRow
            label="Knowledge"
            value={knowledgeHealth.value}
            detail={knowledgeHealth.detail}
            tone={knowledgeHealth.tone}
          />
          <SignalRow
            label="Daemon"
            value={daemonHealth.value}
            detail={daemonHealth.detail}
            tone={daemonHealth.tone}
          />
        </View>
      </View>

      <View style={styles.section}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>
          Diagnostics
        </Text>
        <View style={[styles.sectionBody, { borderTopColor: t.border }]}>
          <DetailRow label="Home state" value={status?.home_status ?? "idle"} />
          <DetailRow
            label="Daemon uptime"
            value={formatElapsedSeconds(daemon?.uptime_secs)}
          />
          <DetailRow label="Tick interval" value={cadence.tickIntervalLabel} />
          <DetailRow label="Tick budget" value={cadence.tickBudgetLabel} />
          <DetailRow
            label="Latest trace"
            value={formatTimestamp(diagnostics?.latest_trace_at)}
          />
          <DetailRow label="Snapshot coverage" value={knowledgeHealth.value} />
          <DetailRow label="Paused" value={String(status?.paused_count ?? 0)} />
          <DetailRow label="Failed" value={String(status?.failed_count ?? 0)} />
          <DetailRow label="Scheduled" value={String(status?.scheduled_count ?? 0)} />
          <DetailRow label="High priority" value={String(status?.high_priority_count ?? 0)} />
          <Text style={[styles.helperText, { color: t.mutedForeground }]}>
            Recoverable sessions can be resumed without another user prompt when the daemon is restarted.
          </Text>
        </View>
        <View style={styles.actionRow}>
          <Pressable
            onPress={() => {
              void state.recoverDaemon();
            }}
            disabled={state.isRecovering}
            style={styles.inlineAction}
          >
            <Text
              style={[
                styles.inlineActionText,
                {
                  color:
                    daemon?.recoverable_session_count || state.isRecovering
                      ? t.userMessage
                      : t.mutedForeground,
                },
              ]}
            >
              {state.isRecovering ? "Recovering..." : "Recover daemon"}
            </Text>
          </Pressable>
        </View>
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
  diagnostics?: MakoDiagnosticsSummary | null,
) {
  if (diagnostics) {
    switch (diagnostics.queue_pressure) {
      case "attention":
        return {
          value: "Attention",
          detail: `${diagnostics.attention_run_count} runs need intervention • ${approvalCount} approvals waiting • ${diagnostics.due_soon_wake_count} wake within 1h`,
          tone: "warning" as const,
        };
      case "busy":
        return {
          value: "Busy",
          detail: `${diagnostics.open_run_count} open runs are moving • ${diagnostics.due_soon_wake_count} wake within 1h`,
          tone: "accent" as const,
        };
      default:
        return {
          value: "Calm",
          detail: `${diagnostics.open_run_count} open runs • ${diagnostics.due_soon_wake_count} near-term wakes`,
          tone: "success" as const,
        };
    }
  }

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

function formatHealthState(state?: MakoHealthState | null): string {
  switch (state) {
    case "healthy":
      return "Healthy";
    case "attention":
      return "Attention";
    case "degraded":
      return "Degraded";
    default:
      return "Pending";
  }
}

function formatElapsedSeconds(value?: number | null): string {
  if (!value || value < 60) {
    return `${value ?? 0}s`;
  }
  const minutes = Math.floor(value / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  return `${days}d`;
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

function summarizeRuntimeDrift(
  runs: MakoCurrentRunSummary[],
  diagnostics?: MakoDiagnosticsSummary | null,
) {
  if (diagnostics) {
    if (diagnostics.stalled_count === 0 && diagnostics.overdue_wake_count === 0) {
      return {
        value: "Healthy",
        detail: "No runs are currently drifting or overdue.",
        tone: "success" as const,
      };
    }

    const staleCount = Math.max(
      diagnostics.stalled_count - diagnostics.overdue_wake_count,
      0,
    );

    return {
      value: `${diagnostics.stalled_count} drifting`,
      detail: `${diagnostics.overdue_wake_count} overdue wakes • ${staleCount} stale active or queued runs`,
      tone:
        diagnostics.overdue_wake_count > 0
          ? ("warning" as const)
          : diagnostics.repeating_failure_count > 0
            ? ("danger" as const)
            : ("accent" as const),
    };
  }

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

function summarizeKnowledgeHealth(
  health?: MakoKnowledgeHealthSummary | null,
) {
  if (!health || health.scope_count === 0) {
    return {
      value: "Pending",
      detail: "Knowledge snapshots will appear after Mako has enough workspace history to consolidate.",
      tone: "default" as const,
    };
  }

  if (health.missing_snapshot_count > 0) {
    return {
      value: `${health.healthy_scope_count}/${health.scope_count}`,
      detail: `${health.missing_snapshot_count} workspace snapshots are still missing.`,
      tone: "warning" as const,
    };
  }

  if (health.stale_snapshot_count > 0) {
    return {
      value: `${health.healthy_scope_count}/${health.scope_count}`,
      detail: `${health.stale_snapshot_count} workspace snapshots are behind the latest reports or runs.`,
      tone: "accent" as const,
    };
  }

  return {
    value: `${health.healthy_scope_count}/${health.scope_count}`,
    detail: `All workspace snapshots are current. Latest snapshot ${formatTimestamp(
      health.latest_snapshot_at,
    )}.`,
    tone: "success" as const,
  };
}

function summarizeDaemonHealth(
  daemon?: MakoDaemonSummary | null,
  healthState?: MakoHealthState | null,
) {
  if (!daemon) {
    return {
      value: "Pending",
      detail: "Daemon stats will appear once Mako has loaded current workspace state.",
      tone: "default" as const,
    };
  }

  const detail = `${daemon.active_runtime_count} active • ${daemon.scheduled_wake_count} scheduled • ${daemon.event_stream_count} streams • ${daemon.recoverable_session_count} recoverable`;
  if (healthState === "degraded") {
    return {
      value: formatElapsedSeconds(daemon.uptime_secs),
      detail,
      tone: "danger" as const,
    };
  }
  if (daemon.recoverable_session_count > 0 || daemon.scheduled_wake_count > 0) {
    return {
      value: formatElapsedSeconds(daemon.uptime_secs),
      detail,
      tone: "accent" as const,
    };
  }
  return {
    value: formatElapsedSeconds(daemon.uptime_secs),
    detail,
    tone: "success" as const,
  };
}

function SummaryCell({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.summaryCell}>
      <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.summaryValue, { color: t.foreground }]}>{value}</Text>
    </View>
  );
}

function DetailRow({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.detailRow, { borderBottomColor: t.border }]}>
      <Text style={[styles.detailLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.detailValue, { color: t.foreground }]}>{value}</Text>
    </View>
  );
}

function SignalRow({
  label,
  value,
  detail,
  tone = "default",
}: {
  label: string;
  value: string;
  detail: string;
  tone?: "default" | "accent" | "warning" | "danger" | "success";
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const valueColor = (() => {
    switch (tone) {
      case "accent":
        return t.userMessage;
      case "warning":
        return t.warning;
      case "danger":
        return t.error;
      case "success":
        return t.success;
      default:
        return t.foreground;
    }
  })();

  return (
    <View style={[styles.signalRow, { borderBottomColor: t.border }]}>
      <View style={styles.signalCopy}>
        <Text style={[styles.signalLabel, { color: t.foreground }]}>{label}</Text>
        <Text style={[styles.signalDetail, { color: t.mutedForeground }]}>
          {detail}
        </Text>
      </View>
      <Text style={[styles.signalValue, { color: valueColor }]}>{value}</Text>
    </View>
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
  summaryStrip: {
    flexDirection: "row",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  summaryCell: {
    flex: 1,
    minHeight: 64,
    justifyContent: "center",
    paddingHorizontal: 8,
    paddingVertical: 10,
  },
  summaryLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
    textAlign: "center",
  },
  summaryValue: {
    marginTop: 6,
    fontSize: 17,
    fontWeight: "700",
    letterSpacing: -0.3,
    textAlign: "center",
  },
  section: {
    gap: 8,
  },
  sectionBody: {
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  signalRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  signalCopy: {
    flex: 1,
    gap: 3,
  },
  signalLabel: {
    fontSize: 14,
    fontWeight: "600",
  },
  signalDetail: {
    fontSize: 12,
    lineHeight: 17,
  },
  signalValue: {
    fontSize: 13,
    fontWeight: "700",
    lineHeight: 18,
    textAlign: "right",
    maxWidth: 116,
  },
  detailRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  detailLabel: {
    fontSize: 13,
    lineHeight: 18,
  },
  detailValue: {
    fontSize: 13,
    fontWeight: "600",
    lineHeight: 18,
    textAlign: "right",
    maxWidth: 148,
  },
  helperText: {
    paddingTop: 10,
    fontSize: 12,
    lineHeight: 17,
  },
  actionRow: {
    marginTop: 2,
  },
  inlineAction: {
    alignSelf: "flex-start",
    paddingVertical: 6,
  },
  inlineActionText: {
    fontSize: 13,
    fontWeight: "600",
  },
  sectionTitle: {
    fontSize: 16,
    fontWeight: "700",
    letterSpacing: -0.2,
  },
});
