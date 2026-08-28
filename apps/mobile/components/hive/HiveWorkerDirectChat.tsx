import { useCallback, useEffect, useRef, useState } from "react";
import {
  Keyboard,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { ArrowUp, Square } from "lucide-react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { HiveWorker, ModelInfo } from "@mitsuro/api";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import {
  workerAutonomyLabel,
  workerAvatarColor,
  workerInitials,
} from "./workerAppearance";
import { HiveWorkerGovernorPanel } from "./HiveWorkerGovernorPanel";
import { mergeRejectedWorkerDraft } from "./workerRejectedDraft";

const COMPOSER_HORIZONTAL_PADDING = 12;
const COMPOSER_MIN_INPUT_HEIGHT = 44;
const COMPOSER_MAX_INPUT_HEIGHT = 132;
const MAX_WORKER_DRAFTS = 12;
export const MAX_WORKER_MESSAGE_BYTES = 64 * 1024;
const workerDrafts = new Map<string, string>();

export function workerMessageUtf8Bytes(value: string): number {
  let bytes = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0) ?? 0;
    bytes += codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
      ? 2
      : codePoint <= 0xffff
      ? 3
      : 4;
  }
  return bytes;
}

function rememberWorkerDraft(key: string, value: string) {
  workerDrafts.delete(key);
  if (value) workerDrafts.set(key, value);
  while (workerDrafts.size > MAX_WORKER_DRAFTS) {
    const oldest = workerDrafts.keys().next().value;
    if (!oldest) break;
    workerDrafts.delete(oldest);
  }
}

function exactModelMatches(worker: HiveWorker, model: ModelInfo): boolean {
  const workerKey = worker.model_key;
  const modelKey = model.key;
  return Boolean(
    workerKey &&
      modelKey &&
      workerKey.provider === modelKey.provider &&
      workerKey.model_id === modelKey.model_id &&
      workerKey.api_format === modelKey.api_format &&
      (workerKey.auth_scope ?? null) === (modelKey.auth_scope ?? null),
  );
}

export function workerDirectChatModelLabel(
  worker: HiveWorker,
  models: ModelInfo[],
): string {
  const exactModel = models.find((model) => exactModelMatches(worker, model));
  const modelName = exactModel?.display_name ?? worker.model ??
    "Model unavailable";
  const provider = worker.model_key?.provider ?? exactModel?.provider;
  return provider ? `${modelName} · ${provider}` : modelName;
}

export function workerDirectChatPermissionLabel(worker: HiveWorker): string {
  switch (worker.permission_mode) {
    case "autonomous":
      return "Autonomous permissions";
    case "supervised":
      return "Supervised permissions";
    default: {
      const readable = worker.permission_mode.replace(/[_-]+/g, " ").trim();
      return readable ? `${readable} permissions` : "Permissions unavailable";
    }
  }
}

function workerStatusLabel(worker: HiveWorker): string {
  switch (worker.status) {
    case "active":
      return "Active";
    case "paused":
      return "Paused";
    case "archived":
      return "Archived";
  }
}

interface HiveWorkerDirectChatHeaderProps {
  worker: HiveWorker;
  models: ModelInfo[];
}

/**
 * Read-only projection of the durable Worker binding used by every DM turn.
 * These are Worker settings, not per-message composer choices.
 */
export function HiveWorkerDirectChatHeader({
  worker,
  models,
}: HiveWorkerDirectChatHeaderProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const avatarColor = workerAvatarColor(worker);
  const statusLabel = workerStatusLabel(worker);

  return (
    <View
      accessibilityLabel={`${worker.display_name} Worker conversation`}
      style={[styles.header, { borderBottomColor: t.border }]}
    >
      <View style={styles.identityRow}>
        <View style={[styles.avatar, { backgroundColor: avatarColor }]}>
          <Text style={[styles.avatarText, { color: t.onAccent }]}>
            {workerInitials(worker.display_name)}
          </Text>
        </View>
        <View style={styles.identityCopy}>
          <Text
            numberOfLines={1}
            style={[styles.workerName, { color: t.foreground }]}
          >
            {worker.display_name}
          </Text>
          <Text
            numberOfLines={1}
            style={[styles.workerKind, { color: t.mutedForeground }]}
          >
            Private Worker chat · {workerAutonomyLabel(worker)}
          </Text>
        </View>
        <View style={styles.statusWrap}>
          <View
            style={[
              styles.statusDot,
              {
                backgroundColor: worker.status === "active"
                  ? avatarColor
                  : t.mutedForeground,
              },
            ]}
          />
          <Text style={[styles.statusText, { color: t.mutedForeground }]}>
            {statusLabel}
          </Text>
        </View>
      </View>

      <View style={styles.settingsBlock}>
        <Text
          numberOfLines={1}
          style={[styles.settingsValue, { color: t.foreground }]}
        >
          {workerDirectChatModelLabel(worker, models)}
        </Text>
        <Text
          numberOfLines={1}
          style={[styles.settingsValue, { color: t.foreground }]}
        >
          {workerDirectChatPermissionLabel(worker)}
        </Text>
        <Text style={[styles.settingsHint, { color: t.mutedForeground }]}>
          Model and permissions are pinned in Worker settings.
        </Text>
      </View>
      <HiveWorkerGovernorPanel
        worker={worker}
        sessionId={worker.dm_session_id ?? null}
        enabled={Boolean(worker.dm_session_id)}
        compact
      />
    </View>
  );
}

interface HiveWorkerComposerProps {
  worker: HiveWorker;
  sessionId: string;
  disabled: boolean;
  isStreaming: boolean;
  onSend: (sessionId: string, content: string) => Promise<void>;
  onStop: (sessionId: string) => void;
  onHeightChange: (height: number) => void;
}

function useWorkerComposerKeyboardLift(): number {
  const [lift, setLift] = useState(0);

  useEffect(() => {
    if (Platform.OS === "web") return;
    const showEvent = Platform.OS === "ios"
      ? "keyboardWillShow"
      : "keyboardDidShow";
    const changeEvent = Platform.OS === "ios"
      ? "keyboardWillChangeFrame"
      : "keyboardDidShow";
    const hideEvent = Platform.OS === "ios"
      ? "keyboardWillHide"
      : "keyboardDidHide";
    const apply = (height: number) => setLift(Math.max(0, Math.round(height)));
    const show = Keyboard.addListener(showEvent, (event) => {
      apply(event.endCoordinates.height);
    });
    const change = Keyboard.addListener(changeEvent, (event) => {
      apply(event.endCoordinates.height);
    });
    const hide = Keyboard.addListener(hideEvent, () => apply(0));
    return () => {
      show.remove();
      change.remove();
      hide.remove();
    };
  }, []);

  return lift;
}

/** Text-only composer for a Worker's serialized private DM lane. */
export function HiveWorkerComposer({
  worker,
  sessionId,
  disabled,
  isStreaming,
  onSend,
  onStop,
  onHeightChange,
}: HiveWorkerComposerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const insets = useSafeAreaInsets();
  const keyboardLift = useWorkerComposerKeyboardLift();
  const draftKey = `worker:${worker.id}:dm:${sessionId}`;
  const [text, setText] = useState(() => workerDrafts.get(draftKey) ?? "");
  const [messageBytes, setMessageBytes] = useState(() =>
    workerMessageUtf8Bytes((workerDrafts.get(draftKey) ?? "").trim())
  );
  const [inputHeight, setInputHeight] = useState(COMPOSER_MIN_INPUT_HEIGHT);
  const [isSending, setIsSending] = useState(false);
  const textRef = useRef(text);
  const sendInFlightRef = useRef(false);

  useEffect(
    () => () => {
      rememberWorkerDraft(draftKey, textRef.current);
      onHeightChange(0);
    },
    [draftKey, onHeightChange],
  );

  const handleTextChange = useCallback((value: string) => {
    textRef.current = value;
    setText(value);
    setMessageBytes(workerMessageUtf8Bytes(value.trim()));
    rememberWorkerDraft(draftKey, value);
  }, [draftKey]);

  const handleAction = useCallback(() => {
    if (isStreaming) {
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
      onStop(sessionId);
      return;
    }
    const content = textRef.current.trim();
    const contentBytes = workerMessageUtf8Bytes(content);
    if (
      disabled || sendInFlightRef.current || !content ||
      contentBytes > MAX_WORKER_MESSAGE_BYTES
    ) return;

    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    sendInFlightRef.current = true;
    setIsSending(true);
    textRef.current = "";
    setText("");
    setMessageBytes(0);
    setInputHeight(COMPOSER_MIN_INPUT_HEIGHT);
    rememberWorkerDraft(draftKey, "");
    void onSend(sessionId, content)
      .catch(() => {
        const restoredDraft = mergeRejectedWorkerDraft(
          content,
          textRef.current,
        );
        textRef.current = restoredDraft;
        setText(restoredDraft);
        setMessageBytes(workerMessageUtf8Bytes(restoredDraft.trim()));
        rememberWorkerDraft(draftKey, restoredDraft);
      })
      .finally(() => {
        sendInFlightRef.current = false;
        setIsSending(false);
      });
  }, [disabled, draftKey, isStreaming, onSend, onStop, sessionId]);

  const messageTooLarge = messageBytes > MAX_WORKER_MESSAGE_BYTES;
  const canSend = !disabled && !isSending && text.trim().length > 0 &&
    !messageTooLarge;
  const placeholder = worker.status === "paused"
    ? `${worker.display_name} is paused`
    : worker.status === "archived"
    ? `${worker.display_name} is archived`
    : `Message ${worker.display_name}…`;

  return (
    <View
      style={[
        styles.composerRoot,
        {
          backgroundColor: t.background,
          borderTopColor: t.border,
          bottom: keyboardLift,
          paddingBottom: Math.max(insets.bottom, 10),
        },
      ]}
      onLayout={(event) => {
        onHeightChange(Math.ceil(event.nativeEvent.layout.height));
      }}
    >
      <View
        style={[
          styles.composerBar,
          {
            backgroundColor: t.glass.background,
            borderColor: t.glass.border,
          },
        ]}
      >
        <TextInput
          accessibilityLabel={`Message ${worker.display_name}`}
          editable={!disabled && !isSending}
          keyboardAppearance={theme.scheme}
          maxLength={MAX_WORKER_MESSAGE_BYTES}
          multiline
          onChangeText={handleTextChange}
          onContentSizeChange={(event) => {
            setInputHeight(Math.max(
              COMPOSER_MIN_INPUT_HEIGHT,
              Math.min(
                COMPOSER_MAX_INPUT_HEIGHT,
                Math.ceil(event.nativeEvent.contentSize.height),
              ),
            ));
          }}
          placeholder={placeholder}
          placeholderTextColor={`${t.mutedForeground}70`}
          scrollEnabled={inputHeight >= COMPOSER_MAX_INPUT_HEIGHT}
          style={[
            styles.input,
            {
              color: t.foreground,
              height: inputHeight,
            },
          ]}
          value={text}
        />
        <Pressable
          accessibilityLabel={isStreaming
            ? "Stop Worker response"
            : "Send message"}
          accessibilityRole="button"
          disabled={!isStreaming && !canSend}
          onPress={handleAction}
          style={({ pressed }) => [
            styles.sendButton,
            {
              backgroundColor: isStreaming || canSend
                ? t.userMessage
                : `${t.mutedForeground}18`,
              opacity: pressed ? 0.78 : 1,
            },
          ]}
        >
          {isStreaming
            ? <Square size={12} color={t.onAccent} strokeWidth={2.4} />
            : (
              <ArrowUp
                size={19}
                color={canSend ? t.onAccent : t.mutedForeground}
              />
            )}
        </Pressable>
      </View>
      <Text
        accessibilityLiveRegion="polite"
        style={[styles.composerHint, {
          color: messageTooLarge ? t.error : t.mutedForeground,
        }]}
      >
        {messageTooLarge
          ? `Message is ${messageBytes.toLocaleString()} bytes; the limit is ${MAX_WORKER_MESSAGE_BYTES.toLocaleString()}.`
          : `${messageBytes.toLocaleString()} / ${MAX_WORKER_MESSAGE_BYTES.toLocaleString()} bytes · Text uses this Worker’s pinned DM configuration.`}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  header: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    paddingTop: 10,
    paddingBottom: 11,
    gap: 9,
  },
  identityRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  avatar: {
    width: 34,
    height: 34,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarText: {
    fontSize: 12,
    fontWeight: "800",
    letterSpacing: 0.2,
  },
  identityCopy: {
    flex: 1,
    minWidth: 0,
  },
  workerName: {
    fontSize: 15,
    fontWeight: "700",
    letterSpacing: -0.1,
  },
  workerKind: {
    marginTop: 2,
    fontSize: 11,
    fontWeight: "500",
  },
  statusWrap: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
  },
  statusDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  statusText: {
    fontSize: 11,
    fontWeight: "600",
  },
  settingsBlock: {
    paddingLeft: 44,
  },
  settingsValue: {
    fontSize: 11,
    lineHeight: 16,
    fontWeight: "600",
  },
  settingsHint: {
    marginTop: 2,
    fontSize: 10,
    lineHeight: 14,
  },
  composerRoot: {
    position: "absolute",
    left: 0,
    right: 0,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: COMPOSER_HORIZONTAL_PADDING,
    paddingTop: 9,
    zIndex: 20,
  },
  composerBar: {
    minHeight: 54,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 17,
    flexDirection: "row",
    alignItems: "flex-end",
    paddingLeft: 13,
    paddingRight: 7,
    paddingVertical: 5,
    gap: 7,
  },
  input: {
    flex: 1,
    minWidth: 0,
    paddingTop: 11,
    paddingBottom: 9,
    paddingHorizontal: 0,
    fontSize: 16,
    lineHeight: 22,
    textAlignVertical: "top",
  },
  sendButton: {
    width: 36,
    height: 36,
    marginBottom: 4,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  composerHint: {
    marginTop: 5,
    paddingHorizontal: 4,
    fontSize: 10,
    lineHeight: 14,
  },
});
