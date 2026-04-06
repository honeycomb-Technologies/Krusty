import { createLiveActivity } from "expo-widgets";
import {
  Text,
  VStack,
  HStack,
  Spacer,
  Image,
  ProgressView,
  Button,
} from "@expo/ui/swift-ui";
import {
  font,
  foregroundStyle,
  padding,
  cornerRadius,
  background,
  opacity,
  lineLimit,
  frame,
} from "@expo/ui/swift-ui/modifiers";
import type { LiveActivityComponent } from "expo-widgets";

interface ChatStreamProps {
  chatTitle: string;
  model: string;
  status:
    | "streaming"
    | "thinking"
    | "tool_call"
    | "awaiting_approval"
    | "complete";
  currentText: string;
  currentTool: string;
  elapsedSeconds: number;
  tokenCount: number;
  progress: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
  toolApprovalSessionId?: string;
}

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

const ChatStreamActivity: LiveActivityComponent<ChatStreamProps> = (props) => {
  "widget";

  const {
    chatTitle,
    model,
    status,
    currentText,
    currentTool,
    elapsedSeconds,
    tokenCount,
    progress,
    toolApprovalId,
    toolApprovalName,
    toolApprovalSessionId,
  } = props;

  const statusColor =
    status === "streaming"
      ? "#ff6b35"
      : status === "thinking"
        ? "#fbbf24"
        : status === "tool_call"
          ? "#60a5fa"
          : status === "awaiting_approval"
            ? "#ef4444"
            : "#4ade80";

  const statusLabel =
    status === "streaming"
      ? "Streaming"
      : status === "thinking"
        ? "Thinking"
        : status === "tool_call"
          ? currentTool
          : status === "awaiting_approval"
            ? "Approval Required"
            : "Complete";

  return {
    banner: (
      <VStack
        alignment="leading"
        spacing={8}
        modifiers={[padding({ all: 16 })]}
      >
        <HStack>
          <Image
            systemName={
              status === "thinking"
                ? "brain"
                : status === "tool_call"
                  ? "wrench.fill"
                  : status === "awaiting_approval"
                    ? "exclamationmark.shield.fill"
                    : "ellipsis.bubble.fill"
            }
            modifiers={[foregroundStyle(statusColor), font({ size: 18 })]}
          />
          <VStack alignment="leading" spacing={1}>
            <Text
              modifiers={[font({ size: 15, weight: "semibold" }), lineLimit(1)]}
            >
              {chatTitle || "Krusty Chat"}
            </Text>
            <Text modifiers={[font({ size: 12 }), opacity(0.6)]}>
              {model} — {statusLabel}
            </Text>
          </VStack>
          <Spacer />
          <VStack alignment="trailing" spacing={2}>
            <Text
              modifiers={[
                font({ size: 13, weight: "medium", design: "monospaced" }),
                foregroundStyle(statusColor),
              ]}
            >
              {formatElapsed(elapsedSeconds)}
            </Text>
            <Text modifiers={[font({ size: 11 }), opacity(0.5)]}>
              {tokenCount.toLocaleString()} tokens
            </Text>
          </VStack>
        </HStack>

        <Text modifiers={[font({ size: 13 }), lineLimit(3), opacity(0.8)]}>
          {status === "awaiting_approval"
            ? `Tool "${toolApprovalName}" is requesting permission to execute.`
            : status === "tool_call"
              ? `Running tool: ${currentTool}`
              : currentText || "Starting..."}
        </Text>

        <ProgressView
          value={progress}
          modifiers={[foregroundStyle(statusColor)]}
        />

        {status === "awaiting_approval" &&
          toolApprovalId &&
          toolApprovalSessionId && (
          <HStack spacing={12}>
            <Button
              target={`deny:${encodeURIComponent(toolApprovalSessionId)}:${encodeURIComponent(toolApprovalId)}`}
              role="destructive"
              label="Deny"
              modifiers={[frame({ maxWidth: 999 })]}
            />
            <Button
              target={`approve:${encodeURIComponent(toolApprovalSessionId)}:${encodeURIComponent(toolApprovalId)}`}
              label="Approve"
              modifiers={[frame({ maxWidth: 999 })]}
            />
          </HStack>
        )}
      </VStack>
    ),
    compactLeading: (
      <HStack spacing={4}>
        <Image
          systemName={status === "thinking" ? "brain" : "ellipsis.bubble.fill"}
          modifiers={[foregroundStyle(statusColor), font({ size: 12 })]}
        />
      </HStack>
    ),
    compactTrailing: (
      <HStack spacing={4}>
        <Text
          modifiers={[
            font({ size: 11, weight: "medium", design: "monospaced" }),
            foregroundStyle(statusColor),
          ]}
        >
          {Math.round(progress * 100)}%
        </Text>
      </HStack>
    ),
    minimal: (
      <Image
        systemName="ellipsis.bubble.fill"
        modifiers={[foregroundStyle(statusColor), font({ size: 12 })]}
      />
    ),
    expandedCenter: (
      <VStack alignment="leading" spacing={4}>
        <Text
          modifiers={[font({ size: 14, weight: "semibold" }), lineLimit(1)]}
        >
          {chatTitle || "Chat"}
        </Text>
        <Text modifiers={[font({ size: 12 }), lineLimit(2), opacity(0.7)]}>
          {status === "awaiting_approval"
            ? `Tool "${toolApprovalName}" needs approval`
            : status === "tool_call"
              ? `Running: ${currentTool}`
              : currentText || statusLabel}
        </Text>
      </VStack>
    ),
    expandedLeading: (
      <Image
        systemName={
          status === "thinking"
            ? "brain"
            : status === "tool_call"
              ? "wrench.fill"
              : status === "awaiting_approval"
                ? "exclamationmark.shield.fill"
                : "ellipsis.bubble.fill"
        }
        modifiers={[foregroundStyle(statusColor), font({ size: 22 })]}
      />
    ),
    expandedTrailing: (
      <VStack alignment="trailing" spacing={2}>
        <Text
          modifiers={[
            font({ size: 11, design: "monospaced" }),
            foregroundStyle(statusColor),
          ]}
        >
          {formatElapsed(elapsedSeconds)}
        </Text>
        <Text modifiers={[font({ size: 10 }), opacity(0.5)]}>
          {tokenCount.toLocaleString()} tok
        </Text>
      </VStack>
    ),
    expandedBottom: (
      <VStack spacing={6}>
        <HStack>
          <Text modifiers={[font({ size: 10 }), opacity(0.5)]}>{model}</Text>
          <Spacer />
          <Text modifiers={[font({ size: 10 }), foregroundStyle(statusColor)]}>
            {statusLabel}
          </Text>
        </HStack>
        <ProgressView
          value={progress}
          modifiers={[foregroundStyle(statusColor)]}
        />
        {status === "awaiting_approval" && toolApprovalId && (
          <HStack spacing={8}>
            <Button
              target={`deny:${toolApprovalId}`}
              role="destructive"
              label="Deny"
              modifiers={[frame({ maxWidth: 999 })]}
            />
            <Button
              target={`approve:${toolApprovalId}`}
              label="Approve"
              modifiers={[
                frame({ maxWidth: 999 }),
                foregroundStyle("#0b1119"),
                background("#4ade80"),
                cornerRadius(8),
              ]}
            />
          </HStack>
        )}
      </VStack>
    ),
  };
};

export default createLiveActivity("ChatStreamActivity", ChatStreamActivity);
