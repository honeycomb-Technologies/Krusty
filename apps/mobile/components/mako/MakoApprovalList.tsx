import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
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
          <View
            key={approval.tool_call_id}
            style={[
              styles.block,
              {
                borderColor: t.border,
              },
            ]}
          >
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
                  backgroundColor: "transparent",
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
                  styles.secondaryAction,
                  {
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={[styles.secondaryLabel, { color: t.mutedForeground }]}>
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
                  styles.primaryAction,
                  {
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={[styles.primaryLabel, { color: t.success }]}>
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
                  styles.secondaryAction,
                  {
                    opacity: actionsDisabled ? 0.6 : 1,
                  },
                ]}
              >
                <Text style={[styles.secondaryLabel, { color: t.error }]}>
                  {isSubmitting ? "Working..." : "Deny"}
                </Text>
              </Pressable>
            </View>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    gap: 0,
  },
  block: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingVertical: 12,
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
    fontSize: 14,
    fontWeight: "600",
  },
  meta: {
    marginTop: 3,
    fontSize: 12,
    fontWeight: "400",
  },
  toolName: {
    fontSize: 11,
    fontWeight: "600",
  },
  summary: {
    marginTop: 8,
    fontSize: 13,
    lineHeight: 18,
  },
  preview: {
    marginTop: 10,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 10,
  },
  previewText: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
  },
  actions: {
    marginTop: 8,
    flexDirection: "row",
    gap: 14,
  },
  primaryAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  primaryLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  secondaryAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  secondaryLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  empty: {
    fontSize: 14,
    paddingHorizontal: 4,
  },
});
