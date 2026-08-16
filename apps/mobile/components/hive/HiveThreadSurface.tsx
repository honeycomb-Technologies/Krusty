import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { useThemeContext } from "../../hooks/useTheme";
import type { HiveChatContext } from "./types";
import { useHiveSessionView } from "./hooks/useHiveSessionView";

interface HiveThreadSurfaceProps {
  chat: HiveChatContext;
  scrollToMessageId?: string | null;
  onScrollTargetHandled?: () => void;
  showComposer?: boolean;
  externalBottomPadding?: number;
}

export function HiveThreadSurface({
  chat,
  scrollToMessageId,
  onScrollTargetHandled,
  showComposer = true,
  externalBottomPadding = 150,
}: HiveThreadSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);
  const sessionView = useHiveSessionView();

  return (
    <View style={styles.container}>
      <ChatTranscript
        messages={sessionView.messages}
        sessionId={sessionView.sessionId}
        sessionType="hive"
        scrollStateKey={`hive:${sessionView.sessionId ?? "new"}`}
        isStreaming={sessionView.isStreaming}
        isThinking={sessionView.isThinking}
        isLoading={sessionView.isLoading}
        activeToolCallId={chat.activeToolCallId}
        onApproveTool={chat.onApproveTool}
        onDenyTool={chat.onDenyTool}
        onSubmitToolResult={chat.onSubmitToolResult}
        onPlanConfirm={chat.onPlanConfirm}
        bottomPadding={
          showComposer ? composerReserveHeight : externalBottomPadding
        }
        hideJumpToLatest={bottomControlsOpen}
        scrollToMessageId={scrollToMessageId}
        onScrollTargetHandled={onScrollTargetHandled}
      />

      {sessionView.error ? (
        <View
          style={[
            styles.errorBanner,
            {
              borderColor: `${t.error}40`,
              backgroundColor: `${t.error}14`,
            },
          ]}
        >
          <Text style={[styles.errorText, { color: t.error }]}>
            {sessionView.error}
          </Text>
        </View>
      ) : null}

      {showComposer ? (
        <ChatBar
          draftKey="hive"
          onSend={chat.onSend}
          onStop={chat.onStop}
          onHeightChange={setComposerReserveHeight}
          isStreaming={chat.isStreaming}
          disabled={false}
          thinkingLevel={chat.thinkingLevel}
          onThinkingChange={chat.onThinkingChange}
          permissionMode={chat.permissionMode}
          onPermissionModeToggle={chat.onPermissionModeToggle}
          fastModeEnabled={chat.fastModeEnabled}
          fastModeSupported={chat.fastModeSupported}
          onFastModeToggle={chat.onFastModeToggle}
          mode={chat.mode}
          onModeToggle={chat.onModeToggle}
          onModelSelect={chat.onModelSelect}
          model={chat.model ?? null}
          models={chat.models}
          sessionType="hive"
          tokenCount={chat.tokenCount}
          onOverlayOpenChange={setBottomControlsOpen}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    minHeight: 0,
  },
  errorBanner: {
    marginHorizontal: 16,
    marginBottom: 10,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  errorText: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
});
