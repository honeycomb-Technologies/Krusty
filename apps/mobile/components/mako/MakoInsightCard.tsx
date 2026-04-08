import { StyleSheet, Text, type StyleProp, type ViewStyle } from "react-native";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";

interface MakoInsightCardProps {
  label: string;
  value: string;
  detail?: string;
  tone?: "default" | "accent" | "warning" | "danger" | "success";
  style?: StyleProp<ViewStyle>;
}

export function MakoInsightCard({
  label,
  value,
  detail,
  tone = "default",
  style,
}: MakoInsightCardProps) {
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
    <GlassCard style={style}>
      <Text style={[styles.label, { color: t.mutedForeground }]}>{label}</Text>
      <Text style={[styles.value, { color: valueColor }]}>{value}</Text>
      {detail ? (
        <Text style={[styles.detail, { color: t.mutedForeground }]}>{detail}</Text>
      ) : null}
    </GlassCard>
  );
}

const styles = StyleSheet.create({
  label: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  value: {
    marginTop: 10,
    fontSize: 22,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  detail: {
    marginTop: 8,
    fontSize: 12,
    lineHeight: 17,
  },
});
