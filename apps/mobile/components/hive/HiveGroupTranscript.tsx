import { memo, useCallback, useMemo, useRef } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";
import type { HiveGroupMember, HiveGroupMessage } from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import { workerFallbackColor, workerInitials } from "./workerAppearance";

interface HiveGroupTranscriptProps {
  messages: HiveGroupMessage[];
  members: HiveGroupMember[];
}

interface MemberDisplay {
  slug: string;
  displayName: string;
  color: string;
  provider: string | null;
}

const FORMER_MEMBER: MemberDisplay = {
  slug: "former-member",
  displayName: "Former member",
  color: "#8884",
  provider: null,
};

function memberDisplay(member: HiveGroupMember): MemberDisplay {
  return {
    slug: member.slug,
    displayName: member.display_name,
    color: member.avatar_color ?? workerFallbackColor(member.slug),
    provider: member.provider ?? null,
  };
}

const GroupMessageRow = memo(function GroupMessageRow({
  message,
  sender,
  replyExcerpt,
}: {
  message: HiveGroupMessage;
  sender: MemberDisplay | null;
  replyExcerpt: string | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (message.sender_kind === "system") {
    return (
      <View style={styles.systemRow}>
        <Text style={[styles.systemText, { color: t.mutedForeground }]}>
          {message.content}
        </Text>
      </View>
    );
  }

  if (message.sender_kind === "user") {
    return (
      <View style={styles.userRow}>
        <View
          style={[
            styles.bubble,
            styles.userBubble,
            { backgroundColor: `${t.userMessage}1A`, borderColor: `${t.userMessage}33` },
          ]}
        >
          {replyExcerpt ? (
            <Text
              numberOfLines={1}
              style={[styles.replyExcerpt, { color: t.mutedForeground }]}
            >
              {replyExcerpt}
            </Text>
          ) : null}
          <Text style={[styles.messageText, { color: t.foreground }]}>
            {message.content}
          </Text>
        </View>
      </View>
    );
  }

  const display = sender ?? FORMER_MEMBER;
  return (
    <View style={styles.workerRow}>
      <View
        style={[
          styles.avatar,
          { backgroundColor: `${display.color}22`, borderColor: `${display.color}55` },
        ]}
      >
        <Text style={[styles.avatarText, { color: display.color }]}>
          {workerInitials(display.displayName)}
        </Text>
      </View>
      <View style={styles.workerBubbleColumn}>
        <View style={styles.senderLine}>
          <Text style={[styles.senderName, { color: display.color }]} numberOfLines={1}>
            {display.displayName}
          </Text>
          {display.provider ? (
            <View style={[styles.providerBadge, { borderColor: `${display.color}55` }]}>
              <Text style={[styles.providerBadgeText, { color: display.color }]}>
                {display.provider}
              </Text>
            </View>
          ) : null}
        </View>
        <View
          style={[
            styles.bubble,
            styles.workerBubble,
            { backgroundColor: t.surface, borderColor: t.border },
          ]}
        >
          {replyExcerpt ? (
            <Text
              numberOfLines={1}
              style={[styles.replyExcerpt, { color: t.mutedForeground }]}
            >
              {replyExcerpt}
            </Text>
          ) : null}
          <Text style={[styles.messageText, { color: t.foreground }]}>
            {message.content}
          </Text>
        </View>
      </View>
    </View>
  );
});

/**
 * Multi-author room transcript. Deliberately its own component tree: the
 * chat transcript is a single-session single-author stream, while a room
 * renders one bubble per Worker with stable per-message identity. Historical
 * rows are memoized so a new append never re-renders the whole list.
 */
export function HiveGroupTranscript({ messages, members }: HiveGroupTranscriptProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const listRef = useRef<FlatList<HiveGroupMessage>>(null);

  const membersById = useMemo(() => {
    const map = new Map<string, MemberDisplay>();
    for (const member of members) {
      map.set(member.worker_id, memberDisplay(member));
    }
    return map;
  }, [members]);
  const messagesById = useMemo(() => {
    const map = new Map<string, HiveGroupMessage>();
    for (const message of messages) {
      map.set(message.id, message);
    }
    return map;
  }, [messages]);

  const renderItem = useCallback(
    ({ item }: { item: HiveGroupMessage }) => {
      const sender = item.sender_worker_id
        ? (membersById.get(item.sender_worker_id) ?? null)
        : null;
      const replyTarget = item.reply_to_message_id
        ? messagesById.get(item.reply_to_message_id)
        : undefined;
      const replyExcerpt = replyTarget
        ? `↩ #${replyTarget.seq} · ${replyTarget.content.slice(0, 80)}`
        : null;
      return (
        <GroupMessageRow message={item} sender={sender} replyExcerpt={replyExcerpt} />
      );
    },
    [membersById, messagesById],
  );

  return (
    <FlatList
      ref={listRef}
      data={messages}
      keyExtractor={(message) => message.id}
      renderItem={renderItem}
      contentContainerStyle={styles.listContent}
      onContentSizeChange={() => {
        listRef.current?.scrollToEnd({ animated: true });
      }}
      ListEmptyComponent={
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
          No messages yet. Say something to put the room to work.
        </Text>
      }
    />
  );
}

const styles = StyleSheet.create({
  listContent: {
    paddingHorizontal: 14,
    paddingVertical: 12,
    gap: 10,
  },
  systemRow: {
    alignItems: "center",
    paddingVertical: 2,
  },
  systemText: {
    fontSize: 12,
    fontStyle: "italic",
    textAlign: "center",
  },
  userRow: {
    flexDirection: "row",
    justifyContent: "flex-end",
  },
  workerRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
    paddingRight: 32,
  },
  workerBubbleColumn: {
    flexShrink: 1,
    gap: 3,
  },
  senderLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  senderName: {
    fontSize: 12,
    fontWeight: "700",
  },
  providerBadge: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 6,
    paddingHorizontal: 5,
    paddingVertical: 1,
  },
  providerBadgeText: {
    fontSize: 10,
    fontWeight: "600",
  },
  avatar: {
    width: 28,
    height: 28,
    borderRadius: 14,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 16,
  },
  avatarText: {
    fontSize: 11,
    fontWeight: "700",
  },
  bubble: {
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 8,
    gap: 3,
  },
  userBubble: {
    maxWidth: "82%",
  },
  workerBubble: {
    alignSelf: "flex-start",
  },
  replyExcerpt: {
    fontSize: 11,
  },
  messageText: {
    fontSize: 14,
    lineHeight: 20,
  },
  emptyText: {
    fontSize: 13,
    textAlign: "center",
    marginTop: 32,
  },
});
