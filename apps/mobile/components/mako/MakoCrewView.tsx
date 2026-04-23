import {
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoPresenceDetails } from "./MakoPresenceDetails";
import type { MakoHomeState } from "./types";

interface MakoCrewViewProps {
  state: MakoHomeState;
}

export function MakoCrewView({ state }: MakoCrewViewProps) {
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
        Crew is Mako&apos;s living roster. Each member carries its own identity, soul, and memory, and can be assigned directly into runs when the work needs a distinct presence.
      </Text>

      <MakoPresenceDetails state={state} showChannelsRow={false} />
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
