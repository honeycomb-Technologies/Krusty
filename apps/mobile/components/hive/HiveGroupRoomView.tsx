import { useMemo, useState } from "react";
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { ArrowLeft, ArrowUp, Square } from "lucide-react-native";
import type { HiveGroupMember, HiveGroupTurn } from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import * as Haptics from "../../platform/haptics";
import { HiveGroupTranscript } from "./HiveGroupTranscript";
import { useHiveGroupRoom } from "./hooks/useHiveGroupRoom";
import { workerFallbackColor } from "./workerAppearance";

interface HiveGroupRoomViewProps {
  groupId: string;
  onBack?: () => void;
}

const MODE_LABELS: Record<string, string> = {
  workbench: "Workbench",
  roundtable: "Roundtable",
  direct: "Direct",
};

const MEMBER_STATUS_LABELS: Record<string, string> = {
  dispatched: "queued",
  queued: "queued",
  working: "thinking",
  sleeping: "sleeping",
  retrying: "retrying",
  awaiting_input: "waiting",
  succeeded: "done",
  failed: "failed",
  cancelled: "stopped",
  cancelling: "stopping",
};

/** The trailing "@prefix" of the draft, if the caret sits inside a mention. */
export function activeMentionPrefix(draft: string): string | null {
  const match = draft.match(/(?:^|[\s.,;:!?])@([a-z0-9\-_]*)$/i);
  return match ? match[1].toLowerCase() : null;
}

function memberStatusFor(
  turn: HiveGroupTurn | null,
  member: HiveGroupMember,
  postedWorkerIds: Set<string>,
): string | null {
  if (!turn || turn.status !== "running") return null;
  const outcome = turn.member_outcomes?.[member.worker_id];
  if (!outcome) {
    return turn.speaker_plan.includes(member.worker_id) ? "queued" : null;
  }
  if (
    postedWorkerIds.has(member.worker_id) &&
    (outcome.status === "working" || outcome.status === "succeeded")
  ) {
    return "posted";
  }
  return MEMBER_STATUS_LABELS[outcome.status] ?? outcome.status;
}

/**
 * One group room: multi-author transcript, live member status while a turn
 * runs, and a composer with @mention autocomplete from the roster. Mounted
 * only while the room is open; the room event tail lives in its hook and
 * ends when this surface closes.
 */
export function HiveGroupRoomView({ groupId, onBack }: HiveGroupRoomViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const room = useHiveGroupRoom(groupId, true);
  const [draft, setDraft] = useState("");

  const members = useMemo(
    () => room.detail?.members ?? [],
    [room.detail?.members],
  );
  const turn = room.turn;
  const turnRunning = turn?.status === "running";
  const postedWorkerIds = useMemo(() => {
    if (!turn) return new Set<string>();
    const posted = new Set<string>();
    for (const message of room.messages) {
      if (message.turn_id === turn.id && message.sender_worker_id) {
        posted.add(message.sender_worker_id);
      }
    }
    return posted;
  }, [room.messages, turn]);

  const mentionPrefix = activeMentionPrefix(draft);
  const mentionSuggestions = useMemo(() => {
    if (mentionPrefix === null) return [];
    return members
      .filter(
        (member) =>
          member.status !== "archived" && member.slug.startsWith(mentionPrefix),
      )
      .slice(0, 5);
  }, [members, mentionPrefix]);

  const applyMention = (slug: string) => {
    setDraft((current) =>
      current.replace(/@([a-z0-9\-_]*)$/i, `@${slug} `),
    );
  };

  const handleSend = () => {
    const message = draft.trim();
    if (!message || room.isSending) return;
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setDraft("");
    void room.send(message).catch(() => {
      // Restore the draft so a failed send is never lost.
      setDraft(message);
    });
  };

  const statusLine = useMemo(() => {
    if (!turn) return null;
    const mode = MODE_LABELS[turn.execution_mode] ?? turn.execution_mode;
    if (turn.status === "running") {
      const total = turn.speaker_plan.length;
      const progress =
        turn.execution_mode === "roundtable" && total > 0
          ? ` · speaker ${Math.min(turn.next_speaker_index, total)}/${total}`
          : "";
      return `${mode} turn running${progress}`;
    }
    return `${mode} turn ${turn.status}`;
  }, [turn]);

  return (
    <KeyboardAvoidingView
      style={styles.container}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
    >
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        {onBack ? (
          <Pressable
            onPress={onBack}
            accessibilityRole="button"
            accessibilityLabel="Back to Groups"
            style={styles.headerButton}
          >
            <ArrowLeft size={18} color={t.mutedForeground} strokeWidth={1.8} />
          </Pressable>
        ) : null}
        <View style={styles.headerCopy}>
          <Text style={[styles.headerTitle, { color: t.foreground }]} numberOfLines={1}>
            {room.detail?.title ?? "Group"}
          </Text>
          <Text style={[styles.headerMeta, { color: t.mutedForeground }]} numberOfLines={1}>
            {members.length} Worker{members.length === 1 ? "" : "s"} ·{" "}
            {MODE_LABELS[room.detail?.execution_mode ?? "workbench"]}
          </Text>
        </View>
        {turnRunning ? (
          <Pressable
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
              void room.stop();
            }}
            disabled={room.isStopping}
            accessibilityRole="button"
            accessibilityLabel="Stop the group turn"
            style={[styles.stopButton, { borderColor: t.error }]}
          >
            <Square size={12} color={t.error} strokeWidth={2.4} />
            <Text style={[styles.stopText, { color: t.error }]}>
              {room.isStopping ? "Stopping…" : "Stop"}
            </Text>
          </Pressable>
        ) : null}
      </View>

      {turnRunning ? (
        <View style={[styles.memberStrip, { borderBottomColor: t.border }]}>
          {members.map((member) => {
            const status = memberStatusFor(turn, member, postedWorkerIds);
            if (!status) return null;
            const color = member.avatar_color ?? workerFallbackColor(member.slug);
            const failed = status === "failed";
            return (
              <View
                key={member.worker_id}
                style={[styles.memberChip, { borderColor: `${color}55` }]}
              >
                <View style={[styles.memberDot, { backgroundColor: failed ? t.error : color }]} />
                <Text style={[styles.memberChipText, { color: t.foreground }]} numberOfLines={1}>
                  @{member.slug} · {status}
                </Text>
              </View>
            );
          })}
        </View>
      ) : null}

      {room.error ? (
        <Text style={[styles.errorText, { color: t.error }]}>{room.error}</Text>
      ) : null}

      {room.isLoading && room.messages.length === 0 ? (
        <View style={styles.loading}>
          <ActivityIndicator color={t.mutedForeground} />
        </View>
      ) : (
        <HiveGroupTranscript messages={room.messages} members={members} />
      )}

      {statusLine ? (
        <Text style={[styles.statusLine, { color: t.mutedForeground }]}>{statusLine}</Text>
      ) : null}

      {mentionSuggestions.length > 0 ? (
        <View style={[styles.mentionBar, { borderTopColor: t.border }]}>
          {mentionSuggestions.map((member) => (
            <Pressable
              key={member.worker_id}
              onPress={() => applyMention(member.slug)}
              style={[styles.mentionChip, { backgroundColor: t.surface, borderColor: t.border }]}
            >
              <Text style={[styles.mentionChipText, { color: t.foreground }]}>
                @{member.slug}
              </Text>
            </Pressable>
          ))}
        </View>
      ) : null}

      <View style={[styles.composer, { borderTopColor: t.border }]}>
        <TextInput
          value={draft}
          onChangeText={setDraft}
          placeholder="Message the room — @mention Workers"
          placeholderTextColor={t.mutedForeground}
          multiline
          style={[
            styles.input,
            { color: t.foreground, backgroundColor: t.surface, borderColor: t.border },
          ]}
        />
        <Pressable
          onPress={handleSend}
          disabled={room.isSending || draft.trim().length === 0}
          accessibilityRole="button"
          accessibilityLabel="Send to the group"
          style={[
            styles.sendButton,
            {
              backgroundColor:
                room.isSending || draft.trim().length === 0
                  ? `${t.userMessage}55`
                  : t.userMessage,
            },
          ]}
        >
          <ArrowUp size={16} color={t.onAccent} strokeWidth={2.4} />
        </Pressable>
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerButton: {
    padding: 4,
  },
  headerCopy: {
    flex: 1,
    minWidth: 0,
  },
  headerTitle: {
    fontSize: 16,
    fontWeight: "700",
  },
  headerMeta: {
    fontSize: 12,
  },
  stopButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 5,
  },
  stopText: {
    fontSize: 12,
    fontWeight: "700",
  },
  memberStrip: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  memberChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 8,
    paddingVertical: 3,
  },
  memberDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  memberChipText: {
    fontSize: 11,
    fontWeight: "600",
  },
  errorText: {
    fontSize: 12,
    paddingHorizontal: 14,
    paddingTop: 8,
  },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  statusLine: {
    fontSize: 11,
    paddingHorizontal: 16,
    paddingBottom: 4,
  },
  mentionBar: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
    paddingHorizontal: 14,
    paddingVertical: 6,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  mentionChip: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  mentionChipText: {
    fontSize: 12,
    fontWeight: "600",
  },
  composer: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  input: {
    flex: 1,
    minHeight: 38,
    maxHeight: 120,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 9,
    fontSize: 14,
  },
  sendButton: {
    width: 34,
    height: 34,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
  },
});
