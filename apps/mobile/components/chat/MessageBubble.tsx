import { useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { Brain, ChevronDown, ChevronRight, Clock } from "lucide-react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MarkdownContent } from "./MarkdownContent";
import { ToolCallCard } from "./ToolCallCard";
import { ToolApprovalWidget } from "./ToolApprovalWidget";
import { AskUserQuestionWidget } from "./AskUserQuestionWidget";
import { PlanConfirmWidget } from "./PlanConfirmWidget";
import type { ChatMessage } from "@krusty/api";

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

  const toolCalls = message.toolCalls ?? [];
  const delegatedTools = toolCalls.filter(
    (toolCall) => toolCall.name === "explore" || toolCall.name === "build",
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
      toolCall.name !== "build" &&
      toolCall.name !== "AskUserQuestion" &&
      toolCall.name !== "PlanConfirm" &&
      !INTERNAL_TOOL_NAMES.has(toolCall.name),
  );

  return (
    <View style={[styles.container, isUser && styles.containerUser]}>
      {isUser && message.content.length > 0 && (
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
          <View
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
          </View>
        </View>
      )}

      {!isUser && (
        <>
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

          {message.content.length > 0 && (
            <View style={styles.assistantText}>
              <MarkdownContent content={message.content} />
              {isLast && isStreaming && (
                <View
                  style={[styles.cursor, { backgroundColor: t.userMessage }]}
                />
              )}
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
        </>
      )}
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
    marginVertical: 6,
    gap: 8,
  },
  containerUser: {
    alignItems: "flex-end",
  },
  userWrap: {
    maxWidth: "85%",
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
  assistantText: {
    paddingVertical: 4,
  },
  cursor: {
    width: 2,
    height: 18,
    borderRadius: 1,
    marginTop: 4,
    opacity: 0.8,
  },
  toolSection: {
    gap: 8,
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
    paddingVertical: 4,
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
