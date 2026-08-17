import { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { ChevronDown, ChevronRight } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import type { HiveAttentionItem as HiveAttentionFeedItem } from "./types";
import { formatProjectLabel, formatTimestamp } from "./utils";

interface HiveAttentionItemProps {
  item: HiveAttentionFeedItem;
  onOpenRun?: (runId: string) => void;
  onOpenThread: (item: HiveAttentionFeedItem) => void;
  onApprove?: (sessionId: string, toolCallId: string) => void;
  onDeny?: (sessionId: string, toolCallId: string) => void;
  onMarkRead: (itemId: string, read: boolean) => void;
  onClear: (itemId: string) => void;
}

function kindLabel(item: HiveAttentionFeedItem): string {
  switch (item.kind) {
    case "approval_required":
      return "Approval";
    case "input_required":
      return "Reply";
    case "run_completed":
      return "Run";
    case "run_failed":
      return "Run error";
    case "run_stalled":
      return "Stalled";
    case "scheduled_run_started":
      return "Calendar";
    case "scheduled_run_completed":
      return "Calendar";
    case "delegated_task_completed":
      return "Worker update";
    default:
      return "Update";
  }
}

function canClear(item: HiveAttentionFeedItem): boolean {
  if (!item.active) {
    return true;
  }

  return item.kind === "run_completed";
}

export function HiveAttentionItem({
  item,
  onOpenRun,
  onOpenThread,
  onApprove,
  onDeny,
  onMarkRead,
  onClear,
}: HiveAttentionItemProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState(false);

  const metaLine = useMemo(() => {
    const parts = [kindLabel(item), formatTimestamp(item.createdAt)];
    if (item.projectDir) {
      parts.push(formatProjectLabel(item.projectDir));
    }
    if (item.targetBranch) {
      parts.push(`branch ${item.targetBranch}`);
    }
    return parts.join(" • ");
  }, [item]);

  const handleToggleExpand = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (!item.read) {
      onMarkRead(item.id, true);
    }
    setExpanded((current) => !current);
  };

  return (
    <View style={[styles.container, { borderBottomColor: t.border }]}>
      <Pressable onPress={handleToggleExpand} style={styles.row}>
        <View style={styles.leading}>
          <View
            style={[
              styles.unreadMarker,
              {
                backgroundColor: item.read ? "transparent" : t.userMessage,
                borderColor: item.read ? t.border : t.userMessage,
              },
            ]}
          />
        </View>

        <View style={styles.copy}>
          <Text style={[styles.title, { color: t.foreground }]} numberOfLines={1}>
            {item.title}
          </Text>
          <Text
            style={[styles.summary, { color: t.mutedForeground }]}
            numberOfLines={expanded ? undefined : 2}
          >
            {item.summary}
          </Text>
          <Text style={[styles.meta, { color: t.mutedForeground }]} numberOfLines={1}>
            {metaLine}
          </Text>
        </View>

        {expanded ? (
          <ChevronDown size={16} color={t.mutedForeground} />
        ) : (
          <ChevronRight size={16} color={t.mutedForeground} />
        )}
      </Pressable>

      {expanded ? (
        <View style={styles.expanded}>
          <Text style={[styles.detail, { color: t.foreground }]}>
            {item.detail}
          </Text>

          <View style={styles.actions}>
            {item.kind === "approval_required" && item.sessionId && item.toolCallId ? (
              <>
                <Pressable
                  onPress={() => onApprove?.(item.sessionId!, item.toolCallId!)}
                  style={styles.action}
                >
                  <Text style={[styles.primaryActionLabel, { color: t.userMessage }]}>
                    Approve
                  </Text>
                </Pressable>
                <Pressable
                  onPress={() => onDeny?.(item.sessionId!, item.toolCallId!)}
                  style={styles.action}
                >
                  <Text style={[styles.secondaryActionLabel, { color: t.mutedForeground }]}>
                    Deny
                  </Text>
                </Pressable>
              </>
            ) : null}

            {item.runId ? (
              <Pressable
                onPress={() => onOpenRun?.(item.runId!)}
                style={styles.action}
              >
                <Text style={[styles.primaryActionLabel, { color: t.userMessage }]}>
                  Open run
                </Text>
              </Pressable>
            ) : null}

            <Pressable onPress={() => onOpenThread(item)} style={styles.action}>
              <Text style={[styles.secondaryActionLabel, { color: t.mutedForeground }]}>
                Jump to Hive
              </Text>
            </Pressable>

            <Pressable
              onPress={() => onMarkRead(item.id, item.read ? false : true)}
              style={styles.action}
            >
              <Text style={[styles.secondaryActionLabel, { color: t.mutedForeground }]}>
                {item.read ? "Mark unread" : "Mark read"}
              </Text>
            </Pressable>

            {canClear(item) ? (
              <Pressable onPress={() => onClear(item.id)} style={styles.action}>
                <Text style={[styles.secondaryActionLabel, { color: t.mutedForeground }]}>
                  Clear
                </Text>
              </Pressable>
            ) : null}
          </View>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingVertical: 9,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
  },
  leading: {
    paddingTop: 4,
  },
  unreadMarker: {
    width: 8,
    height: 8,
    borderRadius: 2.5,
    borderWidth: 1,
  },
  copy: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 14,
    fontWeight: "600",
  },
  summary: {
    marginTop: 3,
    fontSize: 12,
    lineHeight: 17,
  },
  meta: {
    marginTop: 4,
    fontSize: 11,
  },
  expanded: {
    paddingLeft: 15,
    paddingTop: 8,
  },
  detail: {
    fontSize: 12,
    lineHeight: 17,
  },
  actions: {
    marginTop: 8,
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 12,
  },
  action: {
    minHeight: 20,
    justifyContent: "center",
  },
  primaryActionLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  secondaryActionLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
});
