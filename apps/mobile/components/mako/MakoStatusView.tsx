import { StyleSheet, Text, View } from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { formatTimestamp } from "./utils";
import type { MakoCurrentState } from "./types";

interface MakoStatusViewProps {
  state: MakoCurrentState;
}

export function MakoStatusView({ state }: MakoStatusViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const status = state.current?.status;

  return (
    <View style={styles.wrap}>
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Status keeps the control-plane truth compact: what is awake, what is blocked, and when the next wake is expected.
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
        <StatusCard label="Paused" value={String(status?.paused_count ?? 0)} />
        <StatusCard label="Failed" value={String(status?.failed_count ?? 0)} />
      </View>

      <GlassCard style={styles.card}>
        <Text style={[styles.cardLabel, { color: t.mutedForeground }]}>
          Next wake
        </Text>
        <Text style={[styles.cardValue, { color: t.foreground }]}>
          {formatTimestamp(status?.next_wake_at)}
        </Text>
      </GlassCard>
    </View>
  );
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
  wrap: {
    flex: 1,
    paddingHorizontal: 16,
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
});
