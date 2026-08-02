import { useState } from "react";
import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveInsightCard } from "./HiveInsightCard";
import { HiveRunList } from "./HiveRunList";
import {
  formatTimestamp,
  getAttentionRuns,
  getQueueHeadRuns,
  getRunGroup,
  getRunNextWakeAt,
  getRunPriority,
} from "./utils";
import type { HiveCurrentState } from "./types";

interface HiveRunsViewProps {
  state: HiveCurrentState;
  onSelectRun: (runId: string) => void;
}

export function HiveRunsView({ state, onSelectRun }: HiveRunsViewProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const runs = state.current?.runs ?? [];
  const [activeActionRunId, setActiveActionRunId] = useState<string | null>(
    null,
  );
  const [actionError, setActionError] = useState<string | null>(null);

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

  const handlePauseRun = async (runId: string) => {
    if (!client || activeActionRunId) {
      return;
    }

    setActionError(null);
    setActiveActionRunId(runId);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.pauseHiveSession(runId);
      await state.refresh();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to pause this run.",
      );
    } finally {
      setActiveActionRunId(null);
    }
  };

  const handleResumeRun = async (runId: string) => {
    if (!client || activeActionRunId) {
      return;
    }

    setActionError(null);
    setActiveActionRunId(runId);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      await client.resumeHiveSession(runId);
      await state.refresh();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to wake this run.",
      );
    } finally {
      setActiveActionRunId(null);
    }
  };

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
        Runs are the work ledger. Open a run to see what Hive is doing now, waiting on, or has already finished.
      </Text>

      {actionError ? (
        <Text style={[styles.error, { color: t.error }]}>{actionError}</Text>
      ) : null}

      <View style={styles.grid}>
        <HiveInsightCard
          label="Open runs"
          value={String(queueHead.length)}
          detail={`${groups.active.length} running • ${attentionRuns.length} need attention`}
          style={styles.metricCard}
        />
        <HiveInsightCard
          label="Upcoming wakes"
          value={String(queuedLaterCount)}
          detail={nextWakeAt ? `Next wake ${formatTimestamp(nextWakeAt)}` : "No wake is scheduled."}
          style={styles.metricCard}
          tone="accent"
        />
        <HiveInsightCard
          label="Priority"
          value={String(highPriorityCount)}
          detail="High-priority work rises to the top of the queue."
          style={styles.metricCard}
          tone={highPriorityCount > 0 ? "warning" : "default"}
        />
        <HiveInsightCard
          label="Finished"
          value={String(groups.completed.length)}
          detail="Finished runs stay here for follow-up."
          style={styles.metricCard}
          tone="success"
        />
      </View>

      <Section title="Next up">
        <HiveRunList
          runs={queueHead.slice(0, 6)}
          emptyLabel="Nothing is waiting in the queue."
          onSelectRun={onSelectRun}
          activeActionRunId={activeActionRunId}
          onPauseRun={(runId) => {
            void handlePauseRun(runId);
          }}
          onResumeRun={(runId) => {
            void handleResumeRun(runId);
          }}
        />
      </Section>

      <Section title="Running">
        <HiveRunList
          runs={groups.active}
          emptyLabel="No runs are active."
          onSelectRun={onSelectRun}
          activeActionRunId={activeActionRunId}
          onPauseRun={(runId) => {
            void handlePauseRun(runId);
          }}
          onResumeRun={(runId) => {
            void handleResumeRun(runId);
          }}
        />
      </Section>

      <Section title="Waiting on you">
        <HiveRunList
          runs={groups.waiting}
          emptyLabel="No runs are waiting on you."
          onSelectRun={onSelectRun}
          activeActionRunId={activeActionRunId}
          onPauseRun={(runId) => {
            void handlePauseRun(runId);
          }}
          onResumeRun={(runId) => {
            void handleResumeRun(runId);
          }}
        />
      </Section>

      <Section title="Sleeping">
        <HiveRunList
          runs={groups.sleeping}
          emptyLabel="No runs are sleeping."
          onSelectRun={onSelectRun}
          activeActionRunId={activeActionRunId}
          onPauseRun={(runId) => {
            void handlePauseRun(runId);
          }}
          onResumeRun={(runId) => {
            void handleResumeRun(runId);
          }}
        />
      </Section>

      <Section title="Queued later">
        <HiveRunList
          runs={groups.queued}
          emptyLabel="No runs are queued for later."
          onSelectRun={onSelectRun}
          activeActionRunId={activeActionRunId}
          onPauseRun={(runId) => {
            void handlePauseRun(runId);
          }}
          onResumeRun={(runId) => {
            void handleResumeRun(runId);
          }}
        />
      </Section>

      <Section title="Completed">
        <HiveRunList
          runs={groups.completed}
          emptyLabel="No finished runs yet."
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
  error: {
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
