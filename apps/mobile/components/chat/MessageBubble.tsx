import { useState } from "react";
import { Image, View, Text, Pressable, StyleSheet } from "react-native";
import { Brain, ChevronDown, ChevronRight, Clock } from "lucide-react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MarkdownContent } from "./MarkdownContent";
import { ToolCallCard } from "./ToolCallCard";
import { ToolApprovalWidget } from "./ToolApprovalWidget";
import { AskUserQuestionWidget } from "./AskUserQuestionWidget";
import { PlanConfirmWidget } from "./PlanConfirmWidget";
import { ImagePreviewModal, imagePreviewUri } from "./ImagePreviewModal";
import type { ChatMessage, ChatMessageAttachment } from "@krusty/api";
import * as Clipboard from "../../platform/clipboard";
import * as Haptics from "../../platform/haptics";

const INTERNAL_TOOL_NAMES = new Set([
  "enter_plan_mode",
  "set_work_mode",
  "task_start",
  "task_complete",
  "add_subtask",
  "set_dependency",
]);

interface MessageBubbleProps {
  message: ChatMessage;
  isLast: boolean;
  isStreaming: boolean;
  isThinking?: boolean;
  activeToolCallId?: string | null;
  onApproveTool?: (toolCallId: string) => void;
  onDenyTool?: (toolCallId: string) => void;
  onSubmitToolResult?: (
    toolCallId: string,
    result: string,
  ) => void | Promise<void>;
  onPlanConfirm?: (
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void | Promise<void>;
}

export function MessageBubble({
  message,
  isLast,
  isStreaming,
  isThinking,
  activeToolCallId,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
}: MessageBubbleProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isUser = message.role === "user";
  const [copied, setCopied] = useState(false);

  const toolCalls = message.toolCalls ?? [];
  const delegatedTools = toolCalls.filter(
    (toolCall) =>
      toolCall.name === "agent" ||
      toolCall.name === "explore" ||
      toolCall.name === "plan" ||
      toolCall.name === "verify" ||
      toolCall.name === "build",
  );
  const questionTools = toolCalls.filter(
    (toolCall) => toolCall.name === "AskUserQuestion",
  );
  const planConfirmTools = toolCalls.filter(
    (toolCall) => toolCall.name === "PlanConfirm",
  );
  const standardTools = toolCalls.filter(
    (toolCall) =>
      toolCall.name !== "explore" &&
      toolCall.name !== "plan" &&
      toolCall.name !== "verify" &&
      toolCall.name !== "build" &&
      toolCall.name !== "agent" &&
      toolCall.name !== "AskUserQuestion" &&
      toolCall.name !== "PlanConfirm" &&
      !INTERNAL_TOOL_NAMES.has(toolCall.name),
  );
  const showStreamingPlaceholder =
    !isUser &&
    isLast &&
    isStreaming &&
    !message.content.length &&
    !message.thinking &&
    delegatedTools.length === 0 &&
    standardTools.length === 0 &&
    questionTools.length === 0 &&
    planConfirmTools.length === 0;

  const handleCopy = () => {
    const value = message.content.trim();
    if (!value) {
      return;
    }

    Clipboard.setStringAsync(value);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <View style={[styles.container, isUser && styles.containerUser]}>
      {isUser && (message.content.length > 0 || (message.attachments?.length ?? 0) > 0) && (
        <View
          style={[styles.userWrap, message.isQueued && styles.userQueuedWrap]}
        >
          {message.isQueued && (
            <View style={styles.queuedRow}>
              <Clock size={12} color={t.warning} strokeWidth={2} />
              <Text style={[styles.queuedLabel, { color: t.warning }]}>
                Queued
              </Text>
            </View>
          )}
          {(message.attachments?.length ?? 0) > 0 ? (
            <MessageAttachments attachments={message.attachments ?? []} isUser />
          ) : null}
          {message.content.length > 0 ? (
            <Pressable
              onLongPress={handleCopy}
              delayLongPress={250}
              style={[
                styles.userBubble,
                {
                  backgroundColor: message.isQueued
                    ? `${t.warning}20`
                    : t.userMessage,
                },
              ]}
            >
              <MarkdownContent content={message.content} isUser />
            </Pressable>
          ) : null}
          {copied ? (
            <Text style={[styles.copyStatus, { color: t.mutedForeground }]}>
              Copied
            </Text>
          ) : null}
        </View>
      )}

      {!isUser && (
        <View style={styles.assistantWrap}>
          {(message.thinking || (isLast && isThinking)) && (
            <ThinkingBlock
              content={message.thinking ?? ""}
              isStreaming={
                isLast && (isThinking || (isStreaming && !message.content))
              }
            />
          )}

          {delegatedTools.length > 0 && (
            <View style={styles.toolSection}>
              {delegatedTools.map((toolCall) => (
                <ToolCallCard
                  key={toolCall.id}
                  toolCall={toolCall}
                  isStreaming={isLast && isStreaming}
                />
              ))}
            </View>
          )}

          {standardTools.length > 0 && (
            <View style={styles.toolSection}>
              <Text style={[styles.toolLabel, { color: t.mutedForeground }]}>
                Actions
              </Text>
              {standardTools.map((toolCall) =>
                toolCall.status === "awaiting_approval" &&
                onApproveTool &&
                onDenyTool ? (
                  <ToolApprovalWidget
                    key={toolCall.id}
                    toolCall={toolCall}
                    isSubmitting={activeToolCallId === toolCall.id}
                    onApprove={() => onApproveTool(toolCall.id)}
                    onDeny={() => onDenyTool(toolCall.id)}
                  />
                ) : (
                  <ToolCallCard
                    key={toolCall.id}
                    toolCall={toolCall}
                    isStreaming={isLast && isStreaming}
                  />
                ),
              )}
            </View>
          )}

          {(message.attachments?.length ?? 0) > 0 ? (
            <MessageAttachments attachments={message.attachments ?? []} />
          ) : null}

          {message.content.length > 0 && (
            <Pressable
              onLongPress={handleCopy}
              delayLongPress={250}
              style={styles.assistantText}
            >
              <MarkdownContent content={message.content} />
              {isLast && isStreaming && (
                <View
                  style={[styles.cursor, { backgroundColor: t.userMessage }]}
                />
              )}
            </Pressable>
          )}

          {showStreamingPlaceholder && (
            <View style={styles.assistantText}>
              <Text
                style={[styles.placeholderText, { color: t.mutedForeground }]}
              >
                Replying...
              </Text>
              <View
                style={[styles.cursor, { backgroundColor: t.userMessage }]}
              />
            </View>
          )}

          {questionTools.length > 0 && onSubmitToolResult && (
            <View style={styles.toolSection}>
              {questionTools.map((toolCall) => (
                <AskUserQuestionWidget
                  key={toolCall.id}
                  toolCall={toolCall}
                  isSubmitting={activeToolCallId === toolCall.id}
                  onSubmit={(result) => onSubmitToolResult(toolCall.id, result)}
                />
              ))}
            </View>
          )}

          {planConfirmTools.length > 0 && onPlanConfirm && (
            <View style={styles.toolSection}>
              {planConfirmTools.map((toolCall) => (
                <PlanConfirmWidget
                  key={toolCall.id}
                  toolCall={toolCall}
                  isSubmitting={activeToolCallId === toolCall.id}
                  onConfirm={(choice) => onPlanConfirm(toolCall.id, choice)}
                />
              ))}
            </View>
          )}

          {copied ? (
            <Text style={[styles.copyStatus, { color: t.mutedForeground }]}>
              Copied
            </Text>
          ) : null}
        </View>
      )}
    </View>
  );
}

function MessageAttachments({
  attachments,
  isUser,
}: {
  attachments: ChatMessageAttachment[];
  isUser?: boolean;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [previewAttachment, setPreviewAttachment] =
    useState<ChatMessageAttachment | null>(null);
  const previewUri = imagePreviewUri(previewAttachment);

  return (
    <View style={[styles.attachmentStrip, isUser && styles.attachmentStripUser]}>
      {attachments.map((attachment, index) => {
        const uri = imagePreviewUri(attachment);
        if (attachment.type === "image" && uri) {
          return (
            <Pressable
              key={`${attachment.name ?? "image"}-${index}`}
              onPress={(event) => {
                event.stopPropagation();
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                setPreviewAttachment(attachment);
              }}
              style={({ pressed, hovered }) => [
                styles.messageImageThumb,
                {
                  borderColor: hovered ? t.userMessage : t.border,
                  opacity: pressed ? 0.86 : 1,
                },
              ]}
            >
              <Image
                source={{ uri }}
                style={styles.messageImage}
                resizeMode="cover"
                accessibilityLabel={attachment.name ?? "Image attachment"}
              />
            </Pressable>
          );
        }

        return (
          <View
            key={`${attachment.name ?? "file"}-${index}`}
            style={[
              styles.messageFileChip,
              { borderColor: t.border, backgroundColor: t.card },
            ]}
          >
            <Text
              style={[styles.messageFileName, { color: t.mutedForeground }]}
              numberOfLines={1}
            >
              {attachment.name ?? "Attached file"}
            </Text>
          </View>
        );
      })}
      <ImagePreviewModal
        visible={Boolean(previewAttachment)}
        uri={previewUri}
        title={previewAttachment?.name}
        onClose={() => setPreviewAttachment(null)}
      />
    </View>
  );
}

function ThinkingBlock({
  content,
  isStreaming,
}: {
  content: string;
  isStreaming: boolean;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState(false);

  return (
    <Pressable
      onPress={() => setExpanded(!expanded)}
      style={[styles.thinkingBlock, { borderColor: `${t.thinking}30` }]}
    >
      <View style={styles.thinkingHeader}>
        <Brain size={14} color={t.thinking} strokeWidth={2} />
        <Text style={[styles.thinkingLabel, { color: t.thinking }]}>
          {isStreaming ? "Thinking..." : "Thinking"}
        </Text>
        {expanded ? (
          <ChevronDown size={14} color={t.thinking} />
        ) : (
          <ChevronRight size={14} color={t.thinking} />
        )}
      </View>
      {expanded && (
        <Text
          style={[styles.thinkingContent, { color: t.mutedForeground }]}
          selectable
        >
          {content}
        </Text>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  container: {
    marginTop: 4,
    marginBottom: 10,
  },
  containerUser: {
    alignItems: "flex-end",
  },
  assistantWrap: {
    maxWidth: "92%",
    gap: 10,
  },
  userWrap: {
    maxWidth: "84%",
    gap: 6,
  },
  userQueuedWrap: {
    alignItems: "flex-end",
  },
  queuedRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    justifyContent: "flex-end",
  },
  queuedLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  userBubble: {
    borderRadius: 20,
    borderBottomRightRadius: 6,
    paddingHorizontal: 16,
    paddingVertical: 10,
  },
  attachmentStrip: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    alignItems: "center",
  },
  attachmentStripUser: {
    alignSelf: "flex-end",
    justifyContent: "flex-end",
  },
  messageImageThumb: {
    width: 132,
    height: 96,
    borderRadius: 14,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    backgroundColor: "rgba(255,255,255,0.06)",
  },
  messageImage: {
    width: "100%",
    height: "100%",
  },
  messageFileChip: {
    maxWidth: 180,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    paddingVertical: 7,
  },
  messageFileName: {
    fontSize: 12,
    fontWeight: "600",
  },
  assistantText: {
    paddingLeft: 2,
    paddingVertical: 2,
  },
  copyStatus: {
    fontSize: 12,
    fontWeight: "500",
    marginTop: -2,
    marginLeft: 2,
  },
  placeholderText: {
    fontSize: 14,
    fontWeight: "500",
  },
  cursor: {
    width: 2,
    height: 18,
    borderRadius: 1,
    marginTop: 4,
    opacity: 0.8,
  },
  toolSection: {
    gap: 6,
  },
  toolLabel: {
    fontSize: 12,
    fontWeight: "500",
    marginBottom: 2,
    marginLeft: 4,
  },
  thinkingBlock: {
    borderLeftWidth: 2,
    paddingLeft: 12,
    paddingVertical: 3,
  },
  thinkingHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  thinkingLabel: {
    fontSize: 13,
    fontWeight: "600",
    flex: 1,
  },
  thinkingContent: {
    fontSize: 13,
    lineHeight: 19,
    marginTop: 6,
    fontFamily: "Courier",
  },
});
