import {
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";

interface HiveInsightCardProps {
  label: string;
  value: string;
  detail?: string;
  tone?: "default" | "accent" | "warning" | "danger" | "success";
  style?: StyleProp<ViewStyle>;
}

export function HiveInsightCard({
  label,
  value,
  detail,
  tone = "default",
  style,
}: HiveInsightCardProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const valueColor = (() => {
    switch (tone) {
      case "accent":
        return t.userMessage;
      case "warning":
        return t.warning;
      case "danger":
        return t.error;
      case "success":
        return t.success;
      default:
        return t.foreground;
    }
  })();

  return (
    <View
      style={[
        styles.card,
        {
          borderColor: t.border,
          backgroundColor: "transparent",
        },
        style,
      ]}
    >
      <Text style={[styles.label, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.value, { color: valueColor }]}>{value}</Text>
      {detail ? (
        <Text style={[styles.detail, { color: t.mutedForeground }]}>{detail}</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 11,
  },
  label: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  value: {
    marginTop: 8,
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  detail: {
    marginTop: 6,
    fontSize: 12,
    lineHeight: 16,
  },
});
