import {
  ActivityIndicator,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoRunList } from "./MakoRunList";
import { MakoSetCourseComposer } from "./MakoSetCourseComposer";
import { formatTimestamp, getRunGroup } from "./utils";
import type { MakoCurrentState } from "./types";

interface MakoCurrentViewProps {
  state: MakoCurrentState;
  workspaceDirectory?: string | null;
  model?: string | null;
  onSelectRun: (runId: string) => void;
  onCourseSet: (runId: string) => Promise<void>;
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
    <GlassCard style={styles.metricCard}>
      <Text style={[styles.metricLabel, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.metricValue, { color: t.foreground }]}>{value}</Text>
      {hint ? (
        <Text style={[styles.metricHint, { color: t.mutedForeground }]}>{hint}</Text>
      ) : null}
    </GlassCard>
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

export function MakoCurrentView({
  state,
  workspaceDirectory,
  model,
  onSelectRun,
  onCourseSet,
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
  const status = state.current?.status;

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
      <View style={styles.metricsRow}>
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
      </View>

      <View style={styles.metricsRow}>
        <SummaryCard
          label="Sleeping"
          value={String(status?.sleeping_count ?? 0)}
          hint={formatTimestamp(status?.next_wake_at)}
        />
        <SummaryCard
          label="Idle"
          value={String(status?.idle_count ?? 0)}
          hint="ready for more"
        />
      </View>

      <MakoSetCourseComposer
        projectDir={workspaceDirectory}
        isSubmitting={state.isDispatching}
        onSubmit={async (task) => {
          const runId = await state.setCourse(task, {
            projectDir: workspaceDirectory ?? undefined,
            model: model ?? undefined,
          });
          if (runId) {
            await onCourseSet(runId);
          }
        }}
      />

      {state.error ? (
        <Text style={[styles.error, { color: t.error }]}>{state.error}</Text>
      ) : null}

      <Section title="Waiting on you">
        <MakoRunList
          runs={waitingRuns}
          emptyLabel="Nothing is blocked right now."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Active runs">
        <MakoRunList
          runs={activeRuns}
          emptyLabel="No active runs."
          onSelectRun={onSelectRun}
        />
      </Section>

      <Section title="Sleeping">
        <MakoRunList
          runs={sleepingRuns}
          emptyLabel="No sleeping runs."
          onSelectRun={onSelectRun}
        />
      </Section>
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
  metricsRow: {
    flexDirection: "row",
    gap: 12,
    paddingHorizontal: 16,
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
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.8,
  },
  metricHint: {
    marginTop: 8,
    fontSize: 12,
    lineHeight: 16,
  },
  section: {
    paddingHorizontal: 16,
    gap: 10,
  },
  sectionTitle: {
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  error: {
    paddingHorizontal: 16,
    fontSize: 13,
    lineHeight: 18,
  },
});
