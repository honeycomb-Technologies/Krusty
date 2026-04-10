import { StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { getRuntimeLabel } from "./utils";

interface MakoStatusBadgeProps {
  status: string;
}

function paletteForStatus(status: string, theme: ReturnType<typeof useThemeContext>["theme"]) {
  const t = theme.colors;
  switch (status) {
    case "awake":
    case "running":
    case "in_progress":
      return {
        background: `${t.userMessage}18`,
        border: `${t.userMessage}3a`,
        color: t.userMessage,
      };
    case "sleeping":
      return {
        background: `${t.warning}18`,
        border: `${t.warning}38`,
        color: t.warning,
      };
    case "waiting":
    case "blocked":
      return {
        background: `${t.warning}18`,
        border: `${t.warning}38`,
        color: t.warning,
      };
    case "degraded":
      return {
        background: `${t.error}18`,
        border: `${t.error}38`,
        color: t.error,
      };
    case "paused":
    case "pending":
    case "idle":
      return {
        background: `${t.mutedForeground}14`,
        border: `${t.mutedForeground}22`,
        color: t.mutedForeground,
      };
    case "completed":
      return {
        background: `${t.success}18`,
        border: `${t.success}38`,
        color: t.success,
      };
    case "failed":
    case "error":
      return {
        background: `${t.error}18`,
        border: `${t.error}38`,
        color: t.error,
      };
    default:
      return {
        background: `${t.warning}18`,
        border: `${t.warning}38`,
        color: t.warning,
      };
  }
}

export function MakoStatusBadge({ status }: MakoStatusBadgeProps) {
  const themeContext = useThemeContext();
  const normalizedStatus = getRuntimeLabel(status);
  const palette = paletteForStatus(normalizedStatus, themeContext.theme);

  return (
    <View
      style={[
        styles.badge,
        {
          backgroundColor: palette.background,
          borderColor: palette.border,
        },
      ]}
    >
      <Text style={[styles.label, { color: palette.color }]}>
        {normalizedStatus}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  badge: {
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 8,
    paddingVertical: 4,
  },
  label: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "lowercase",
  },
});
