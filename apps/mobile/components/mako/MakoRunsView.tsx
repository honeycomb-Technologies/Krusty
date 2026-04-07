import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoRunList } from "./MakoRunList";
import { getRunGroup } from "./utils";
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
        Runs are grouped by where they are in the water right now.
      </Text>

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
  sectionTitle: {
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
});
