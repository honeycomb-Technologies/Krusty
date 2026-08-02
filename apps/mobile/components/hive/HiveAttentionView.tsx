import { StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { ListRowsSkeleton } from "../ui/Skeleton";
import { HiveAttentionItem } from "./HiveAttentionItem";
import { useHiveAttention } from "./hooks/useHiveAttention";
import type { HiveChatContext, HiveCurrentState } from "./types";

interface HiveAttentionViewProps {
  state: HiveCurrentState;
  chat: HiveChatContext;
  onSelectRun: (runId: string) => void;
  onOpenThread: (messageId?: string | null) => void;
}

function SectionTitle({ title, count }: { title: string; count: number }) {
  const { theme } = useThemeContext();
  return (
    <View style={styles.sectionTitleRow}>
      <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}>
        {title}
      </Text>
      <Text style={[styles.sectionCount, { color: theme.colors.mutedForeground }]}>
        {count}
      </Text>
    </View>
  );
}

export function HiveAttentionView({
  state,
  chat,
  onSelectRun,
  onOpenThread,
}: HiveAttentionViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const attention = useHiveAttention(state.current, chat.sessionId);

  if (state.isLoading && !state.current) {
    return (
      <View style={styles.loading}>
        <ListRowsSkeleton rows={5} />
      </View>
    );
  }

  const { needsAction, updates } = attention.sections;
  const hasItems = needsAction.length > 0 || updates.length > 0;

  return (
    <View style={styles.container}>
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <Text style={[styles.title, { color: t.foreground }]}>Attention</Text>
        <Text style={[styles.summaryCopy, { color: t.mutedForeground }]}>
          {attention.badgeCount > 0
            ? `${attention.badgeCount} need action`
            : "No action waiting"}
          {" • "}
          {attention.unreadCount > 0
            ? `${attention.unreadCount} unread`
            : "All caught up"}
        </Text>
      </View>

      {state.error ? (
        <Text style={[styles.errorText, { color: t.error }]}>{state.error}</Text>
      ) : null}
      {attention.error ? (
        <Text style={[styles.errorText, { color: t.error }]}>{attention.error}</Text>
      ) : null}

      {!hasItems ? (
        <Text style={[styles.empty, { color: t.mutedForeground }]}>
          Nothing important is waiting right now.
        </Text>
      ) : null}

      <View style={styles.section}>
        <SectionTitle title="Needs action" count={needsAction.length} />
        {needsAction.length === 0 ? (
          <Text style={[styles.emptySection, { color: t.mutedForeground }]}>
            No approvals or blockers need you right now.
          </Text>
        ) : (
          <View>
            {needsAction.map((item) => (
              <HiveAttentionItem
                key={item.id}
                item={item}
                onOpenRun={onSelectRun}
                onOpenThread={(selectedItem) => {
                  attention.markRead(selectedItem.id, true);
                  onOpenThread(selectedItem.threadMessageId ?? null);
                }}
                onApprove={chat.onApproveTool}
                onDeny={chat.onDenyTool}
                onMarkRead={attention.markRead}
                onClear={attention.clearItem}
              />
            ))}
          </View>
        )}
      </View>

      <View style={styles.section}>
        <SectionTitle title="Updates" count={updates.length} />
        {updates.length === 0 ? (
          <Text style={[styles.emptySection, { color: t.mutedForeground }]}>
            Completions and milestone updates will collect here.
          </Text>
        ) : (
          <View>
            {updates.map((item) => (
              <HiveAttentionItem
                key={item.id}
                item={item}
                onOpenRun={onSelectRun}
                onOpenThread={(selectedItem) => {
                  attention.markRead(selectedItem.id, true);
                  onOpenThread(selectedItem.threadMessageId ?? null);
                }}
                onMarkRead={attention.markRead}
                onClear={attention.clearItem}
              />
            ))}
          </View>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    paddingHorizontal: 16,
    paddingBottom: 8,
  },
  loading: {
    flex: 1,
    paddingHorizontal: 16,
    paddingTop: 16,
    gap: 16,
  },
  header: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingTop: 4,
    paddingBottom: 8,
  },
  title: {
    fontSize: 17,
    fontWeight: "600",
  },
  summaryCopy: {
    marginTop: 4,
    fontSize: 12,
    lineHeight: 16,
  },
  errorText: {
    marginTop: 12,
    fontSize: 12,
    lineHeight: 16,
  },
  empty: {
    marginTop: 12,
    fontSize: 13,
    lineHeight: 18,
  },
  emptySection: {
    fontSize: 13,
    lineHeight: 18,
  },
  section: {
    marginTop: 14,
  },
  sectionTitleRow: {
    flexDirection: "row",
    alignItems: "baseline",
    justifyContent: "space-between",
    marginBottom: 6,
  },
  sectionTitle: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.4,
  },
  sectionCount: {
    fontSize: 11,
    fontWeight: "500",
  },
});
