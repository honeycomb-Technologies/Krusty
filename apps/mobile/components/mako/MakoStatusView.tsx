import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoRunList } from "./MakoRunList";
import { formatTimestamp } from "./utils";
import type { MakoCurrentRunSummary, MakoCurrentState } from "./types";

interface MakoStatusViewProps {
  state: MakoCurrentState;
  onSelectRun: (runId: string) => void;
}

export function MakoStatusView({ state, onSelectRun }: MakoStatusViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const status = state.current?.status;
  const runs = state.current?.runs ?? [];
  const cadence = summarizeCadence(runs);
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
