import { View, Text, Pressable, StyleSheet } from "react-native";
import { ShieldAlert, ShieldCheck, ShieldX, Check, X } from "lucide-react-native";
import { useThemeContext } from "../../hooks/useTheme";
import type { ToolCall } from "@mitsuro/api";

interface ToolApprovalWidgetProps {
  toolCall: ToolCall;
  isSubmitting?: boolean;
  onApprove: () => void;
  onDeny: () => void;
}

export function ToolApprovalWidget({
  toolCall,
  isSubmitting = false,
  onApprove,
  onDeny,
}: ToolApprovalWidgetProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isAwaiting = toolCall.status === "awaiting_approval";

  return (
    <View
      style={[
        styles.card,
        { borderColor: `${t.warning}55`, backgroundColor: `${t.warning}10` },
      ]}
    >
      <View
        style={[
          styles.header,
          {
            borderBottomColor: `${t.warning}33`,
            backgroundColor: `${t.warning}18`,
          },
        ]}
      >
        {isAwaiting ? (
          <ShieldAlert size={16} color={t.warning} strokeWidth={1.8} />
        ) : toolCall.status === "error" ? (
          <ShieldX size={16} color={t.error} strokeWidth={1.8} />
        ) : (
          <ShieldCheck size={16} color={t.success} strokeWidth={1.8} />
        )}
        <Text style={[styles.title, { color: isAwaiting ? t.warning : toolCall.status === "error" ? t.error : t.success }]} numberOfLines={1}>
          Permission: {toolCall.name}
        </Text>
        {!isAwaiting && (
          <View style={styles.badgeRow}>
            {toolCall.status === "error" ? (
              <>
                <X size={12} color={t.error} strokeWidth={2.2} />
                <Text style={[styles.badgeText, { color: t.error }]}>
                  Denied
                </Text>
              </>
            ) : (
              <>
                <Check size={12} color={t.success} strokeWidth={2.2} />
                <Text style={[styles.badgeText, { color: t.success }]}>
                  Approved
                </Text>
              </>
            )}
          </View>
        )}
      </View>

      <View style={styles.body}>
        {toolCall.arguments && (
          <View
            style={[
              styles.preview,
              { borderColor: `${t.border}66`, backgroundColor: `${t.card}66` },
            ]}
          >
            <Text
              style={[styles.previewText, { color: t.mutedForeground }]}
              selectable
            >
              {JSON.stringify(toolCall.arguments, null, 2)}
            </Text>
          </View>
        )}

        {isAwaiting && (
          <View style={styles.actions}>
            <Pressable
              onPress={onApprove}
              disabled={isSubmitting}
              style={[
                styles.primaryButton,
                { backgroundColor: t.success, opacity: isSubmitting ? 0.6 : 1 },
              ]}
            >
              <Check size={16} color="#fff" strokeWidth={2.2} />
              <Text style={styles.primaryLabel}>Approve</Text>
            </Pressable>
            <Pressable
              onPress={onDeny}
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
              <X size={16} color={t.foreground} strokeWidth={2.2} />
              <Text style={[styles.secondaryLabel, { color: t.foreground }]}>
                Deny
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
    gap: 12,
  },
  preview: {
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 10,
  },
  previewText: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
  },
  actions: {
    flexDirection: "row",
    gap: 10,
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
    color: "#fff",
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
