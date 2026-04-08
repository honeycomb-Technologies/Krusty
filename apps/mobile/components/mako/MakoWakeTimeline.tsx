import { StyleSheet, Text, View } from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoStatusBadge } from "./MakoStatusBadge";
import { formatTimestamp } from "./utils";
import type { MakoRunWakeEvent } from "@krusty/api";

interface MakoWakeTimelineProps {
  wake: MakoRunWakeEvent[];
}

export function MakoWakeTimeline({ wake }: MakoWakeTimelineProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (wake.length === 0) {
    return (
      <Text style={[styles.empty, { color: t.mutedForeground }]}>
        Wake will fill in as the run moves.
      </Text>
    );
  }

  return (
    <View style={styles.list}>
      {wake.map((event) => (
        <GlassCard key={event.id} style={styles.card}>
          <View style={styles.row}>
            <View style={styles.copy}>
              <Text style={[styles.title, { color: t.foreground }]}>
                {event.title}
              </Text>
              <Text style={[styles.time, { color: t.mutedForeground }]}>
                {formatTimestamp(event.timestamp)}
              </Text>
            </View>
            <MakoStatusBadge status={event.status} />
          </View>

          {event.detail ? (
            <Text style={[styles.detail, { color: t.mutedForeground }]}>
              {event.detail}
            </Text>
          ) : null}
        </GlassCard>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    gap: 10,
  },
  card: {
    marginBottom: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  copy: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 15,
    fontWeight: "700",
  },
  time: {
    marginTop: 4,
    fontSize: 12,
    fontWeight: "500",
  },
  detail: {
    marginTop: 12,
    fontSize: 13,
    lineHeight: 18,
  },
  empty: {
    fontSize: 14,
  },
});
