import { colors } from '@mitsuro/ui';
import { View, Text, Pressable, StyleSheet } from "react-native";
import { FileText, Play, Trash2, Check } from "lucide-react-native";
import { useThemeContext } from "../../hooks/useTheme";
import type { ToolCall } from "@mitsuro/api";

interface PlanConfirmWidgetProps {
  toolCall: ToolCall;
  isSubmitting?: boolean;
  onConfirm: (choice: "execute" | "abandon") => void | Promise<void>;
}

export function PlanConfirmWidget({
  toolCall,
  isSubmitting = false,
  onConfirm,
}: PlanConfirmWidgetProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const title = String(toolCall.arguments?.title ?? "Implementation Plan");
  const taskCount = Number(toolCall.arguments?.task_count ?? 0);
  const hasSubmitted = toolCall.status === "success";

  return (
    <View
      style={[
        styles.card,
        { borderColor: `${t.success}55`, backgroundColor: `${t.success}10` },
      ]}
    >
      <View
        style={[
          styles.header,
          {
            borderBottomColor: `${t.success}33`,
            backgroundColor: `${t.success}18`,
          },
        ]}
      >
        <FileText size={15} color={t.success} strokeWidth={1.8} />
        <Text style={[styles.title, { color: t.success }]}>Plan Ready</Text>
        {hasSubmitted && (
          <View style={styles.badgeRow}>
            <Check size={12} color={t.success} strokeWidth={2.2} />
            <Text style={[styles.badgeText, { color: t.success }]}>
              Confirmed
            </Text>
          </View>
        )}
      </View>

      <View style={styles.body}>
        <Text style={[styles.planTitle, { color: t.foreground }]}>
          {title}
          {taskCount > 0 ? (
            <Text
              style={{ color: t.mutedForeground }}
            >{` (${taskCount} tasks)`}</Text>
          ) : null}
        </Text>
        <Text style={[styles.description, { color: t.mutedForeground }]}>
          Ready to execute this plan? Choose Execute to switch to Build mode and
          continue, or Abandon to discard it.
        </Text>

        {!hasSubmitted && (
          <View style={styles.actions}>
            <Pressable
              onPress={() => void onConfirm("execute")}
              disabled={isSubmitting}
              style={[
                styles.primaryButton,
                { backgroundColor: t.success, opacity: isSubmitting ? 0.6 : 1 },
              ]}
            >
              <Play size={16} color={t.onAccent} strokeWidth={2.2} />
              <Text style={styles.primaryLabel}>Execute</Text>
            </Pressable>
            <Pressable
              onPress={() => void onConfirm("abandon")}
              disabled={isSubmitting}
              style={[
                styles.secondaryButton,
                {
                  borderColor: `${t.border}88`,
                  backgroundColor: `${t.card}88`,
                  opacity: isSubmitting ? 0.6 : 1,
                },
              ]}
            >
              <Trash2 size={16} color={t.foreground} strokeWidth={2} />
              <Text style={[styles.secondaryLabel, { color: t.foreground }]}>
                Abandon
              </Text>
            </Pressable>
          </View>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderRadius: 14,
    borderWidth: 1,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  title: {
    flex: 1,
    fontSize: 13,
    fontWeight: "600",
  },
  badgeRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  badgeText: {
    fontSize: 11,
    fontWeight: "600",
  },
  body: {
    padding: 14,
    gap: 10,
  },
  planTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  description: {
    fontSize: 13,
    lineHeight: 19,
  },
  actions: {
    flexDirection: "row",
    gap: 10,
    marginTop: 4,
  },
  primaryButton: {
    flex: 1,
    minHeight: 44,
    borderRadius: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  primaryLabel: {
    color: colors.onAccent,
    fontSize: 14,
    fontWeight: "600",
  },
  secondaryButton: {
    flex: 1,
    minHeight: 44,
    borderRadius: 10,
    borderWidth: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  secondaryLabel: {
    fontSize: 14,
    fontWeight: "600",
  },
});
