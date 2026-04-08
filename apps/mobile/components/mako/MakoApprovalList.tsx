import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoPendingApproval } from "@krusty/api";
import { formatPriorityLabel } from "./priority";
import { formatProjectLabel, formatTimestamp } from "./utils";

interface MakoApprovalListProps {
  approvals: MakoPendingApproval[];
  activeToolCallId?: string | null;
  emptyLabel: string;
  onSelectRun: (runId: string) => void;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
}

function formatApprovalArguments(value: unknown): string {
  if (value === undefined) {
    return "No arguments";
  }

  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function MakoApprovalList({
  approvals,
  activeToolCallId,
  emptyLabel,
  onSelectRun,
  onApproveTool,
  onDenyTool,
}: MakoApprovalListProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (approvals.length === 0) {
    return (
      <Text style={[styles.empty, { color: t.mutedForeground }]}>{emptyLabel}</Text>
    );
  }

  return (
    <View style={styles.list}>
      {approvals.map((approval) => {
        const isSubmitting = activeToolCallId === approval.tool_call_id;
        const actionsDisabled = activeToolCallId !== null;

        return (
          <GlassCard key={approval.tool_call_id} style={styles.card}>
            <View style={styles.header}>
              <View style={styles.copy}>
                <Text
                  style={[styles.title, { color: t.foreground }]}
                  numberOfLines={1}
                >
                  {approval.session_title || "Untitled run"}
                </Text>
                <Text
                  style={[styles.meta, { color: t.mutedForeground }]}
                  numberOfLines={1}
                >
                  {[
                    formatProjectLabel(approval.project_dir),
                    formatPriorityLabel(approval.priority),
                    formatTimestamp(approval.requested_at),
                  ].join(" • ")}
                </Text>
              </View>
              <Text style={[styles.toolName, { color: t.warning }]}>
                {approval.tool_name}
              </Text>
            </View>

            <Text style={[styles.summary, { color: t.foreground }]}>
              Permission required before this run can continue.
            </Text>

            <View
              style={[
                styles.preview,
                {
                  borderColor: `${t.border}66`,
                  backgroundColor: `${t.card}66`,
                },
              ]}
            >
              <Text
                style={[styles.previewText, { color: t.mutedForeground }]}
                selectable
              >
                {formatApprovalArguments(approval.arguments)}
              </Text>
            </View>

            <View style={styles.actions}>
              <Pressable
                disabled={actionsDisabled}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  onSelectRun(approval.session_id);
                }}
                style={[
                  styles.secondaryButton,
                  {
                    borderColor: `${t.border}88`,
                    backgroundColor: `${t.card}88`,
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={[styles.secondaryLabel, { color: t.foreground }]}>
                  Open run
                </Text>
              </Pressable>

              <Pressable
                disabled={actionsDisabled}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  onApproveTool(approval.session_id, approval.tool_call_id);
                }}
                style={[
                  styles.primaryButton,
                  {
                    backgroundColor: t.success,
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={styles.primaryLabel}>
                  {isSubmitting ? "Approving..." : "Approve"}
                </Text>
              </Pressable>

              <Pressable
                disabled={actionsDisabled}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  onDenyTool(approval.session_id, approval.tool_call_id);
                }}
                style={[
                  styles.secondaryButton,
                  {
                    borderColor: `${t.border}88`,
                    backgroundColor: `${t.card}88`,
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={[styles.secondaryLabel, { color: t.foreground }]}>
                  {isSubmitting ? "Working..." : "Deny"}
                </Text>
              </Pressable>
            </View>
          </GlassCard>
        );
      })}
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
  header: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  copy: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
  },
  meta: {
    marginTop: 4,
    fontSize: 12,
    fontWeight: "500",
  },
  toolName: {
    fontSize: 12,
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  summary: {
    marginTop: 12,
    fontSize: 13,
    lineHeight: 18,
  },
  preview: {
    marginTop: 12,
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
    marginTop: 12,
    flexDirection: "row",
    gap: 10,
  },
  primaryButton: {
    flex: 1,
    minHeight: 42,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  primaryLabel: {
    color: "#fff",
    fontSize: 14,
    fontWeight: "600",
  },
  secondaryButton: {
    flex: 1,
    minHeight: 42,
    borderRadius: 10,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  secondaryLabel: {
    fontSize: 14,
    fontWeight: "600",
  },
  empty: {
    fontSize: 14,
    paddingHorizontal: 4,
  },
});
