import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoInsightCard } from "./MakoInsightCard";
import { MakoRunList } from "./MakoRunList";
import {
  formatTimestamp,
  getAttentionRuns,
  getQueueHeadRuns,
  getRunGroup,
  getRunNextWakeAt,
  getRunPriority,
} from "./utils";
import type { MakoCurrentState } from "./types";

interface MakoRunsViewProps {
  state: MakoCurrentState;
  onSelectRun: (runId: string) => void;
}

export function MakoRunsView({ state, onSelectRun }: MakoRunsViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const runs = state.current?.runs ?? [];

  const groups = {
    active: runs.filter((run) => getRunGroup(run) === "active"),
    waiting: runs.filter((run) => getRunGroup(run) === "waiting"),
    sleeping: runs.filter((run) => getRunGroup(run) === "sleeping"),
    queued: runs.filter((run) => getRunGroup(run) === "queued"),
    completed: runs.filter((run) => getRunGroup(run) === "completed"),
  };
  const attentionRuns = getAttentionRuns(runs);
  const queueHead = getQueueHeadRuns(runs);
  const highPriorityCount = queueHead.filter(
    (run) => getRunPriority(run) === "high",
  ).length;
  const queuedLaterCount = groups.queued.length + groups.sleeping.length;
  const nextWakeAt =
    queueHead
      .map((run) => getRunNextWakeAt(run))
      .filter((value): value is string => Boolean(value))
      .sort()[0] ?? null;

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
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
        Runs keep the open queue visible first, then break the waterline into active, waiting, sleeping, queued, and completed groups.
      </Text>

      <View style={styles.grid}>
        <MakoInsightCard
          label="Open queue"
          value={String(queueHead.length)}
          detail={`${groups.active.length} active • ${attentionRuns.length} need attention`}
          style={styles.metricCard}
        />
        <MakoInsightCard
          label="Queued later"
          value={String(queuedLaterCount)}
          detail={nextWakeAt ? `Next wake ${formatTimestamp(nextWakeAt)}` : "No wake is queued yet."}
          style={styles.metricCard}
          tone="accent"
        />
        <MakoInsightCard
          label="High priority"
          value={String(highPriorityCount)}
          detail="High-priority work floats to the top of the queue."
          style={styles.metricCard}
          tone={highPriorityCount > 0 ? "warning" : "default"}
        />
        <MakoInsightCard
          label="Completed"
          value={String(groups.completed.length)}
          detail="Finished runs stay visible here for quick follow-up."
          style={styles.metricCard}
          tone="success"
        />
      </View>

      <Section title="Queue head">
        <MakoRunList
          runs={queueHead.slice(0, 6)}
          emptyLabel="No open runs are in the queue."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Active">
        <MakoRunList
          runs={groups.active}
          emptyLabel="No active runs."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Waiting">
        <MakoRunList
          runs={groups.waiting}
          emptyLabel="No runs are waiting on you."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Sleeping">
        <MakoRunList
          runs={groups.sleeping}
          emptyLabel="No sleeping runs."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Queued">
        <MakoRunList
          runs={groups.queued}
          emptyLabel="No queued runs."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Completed">
        <MakoRunList
          runs={groups.completed}
          emptyLabel="No completed runs yet."
          onSelectRun={onSelectRun}
        />
      </Section>
    </ScrollView>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  const { theme } = useThemeContext();
  return (
    <View style={styles.section}>
      <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}>
        {title}
      </Text>
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 18,
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  section: {
    gap: 10,
  },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
  },
  metricCard: {
    width: "47%",
    marginBottom: 0,
  },
  sectionTitle: {
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
});
