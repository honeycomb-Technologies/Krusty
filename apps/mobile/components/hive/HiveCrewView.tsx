import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { HivePresenceDetails } from "./HivePresenceDetails";
import type { HiveHomeState } from "./types";

interface HiveCrewViewProps {
  state: HiveHomeState;
}

export function HiveCrewView({ state }: HiveCrewViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

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
        Hive Agents are the living roster. Each agent carries its own identity, voice, and memory, and can be assigned directly to a run when the work needs a distinct presence.
      </Text>

      <HivePresenceDetails state={state} showChannelsRow={false} />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  wrap: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 14,
  },
  description: {
    fontSize: 12,
    lineHeight: 18,
  },
});
