import { useState, type ReactNode } from "react";
import { StyleSheet, Text, View } from "react-native";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoChatContext } from "./types";

interface MakoThreadSurfaceProps {
  chat: MakoChatContext;
  emptyTitle: string;
  emptyBody: string;
  scrollToMessageId?: string | null;
  onScrollTargetHandled?: () => void;
  emptyAccessory?: ReactNode;
  showComposer?: boolean;
  externalBottomPadding?: number;
}

export function MakoThreadSurface({
  chat,
  emptyTitle,
  emptyBody,
  scrollToMessageId,
  onScrollTargetHandled,
  emptyAccessory,
  showComposer = true,
  externalBottomPadding = 150,
}: MakoThreadSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);

  return (
    <View style={styles.container}>
      <ChatTranscript
        key={`mako:${chat.sessionId ?? "new"}`}
        messages={chat.messages}
        sessionId={chat.sessionId}
        sessionType="mako"
        scrollStateKey={`mako:${chat.sessionId ?? "new"}`}
        isStreaming={chat.isStreaming}
        isThinking={chat.isThinking}
        activeToolCallId={chat.activeToolCallId}
        onApproveTool={chat.onApproveTool}
        onDenyTool={chat.onDenyTool}
        onSubmitToolResult={chat.onSubmitToolResult}
        onPlanConfirm={chat.onPlanConfirm}
        bottomPadding={
          showComposer ? composerReserveHeight : externalBottomPadding
        }
        hideJumpToLatest={bottomControlsOpen}
        showPlanTracker={false}
        scrollToMessageId={scrollToMessageId}
        onScrollTargetHandled={onScrollTargetHandled}
        emptyState={
          <View style={styles.emptyState}>
            <Text style={[styles.emptyTitle, { color: t.foreground }]}>
              {emptyTitle}
            </Text>
            <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
              {emptyBody}
            </Text>
            {emptyAccessory ? (
              <View style={styles.emptyAccessory}>{emptyAccessory}</View>
            ) : null}
          </View>
        }
      />

      {chat.error ? (
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
            {chat.error}
          </Text>
        </View>
      ) : null}

      {showComposer ? (
        <ChatBar
          draftKey="mako"
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
          sessionType="mako"
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
  emptyState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 28,
    gap: 10,
  },
  emptyTitle: {
    fontSize: 22,
    fontWeight: "700",
    textAlign: "center",
    letterSpacing: -0.4,
  },
  emptyBody: {
    fontSize: 15,
    lineHeight: 22,
    textAlign: "center",
  },
  emptyAccessory: {
    width: "100%",
    maxWidth: 520,
    marginTop: 10,
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
