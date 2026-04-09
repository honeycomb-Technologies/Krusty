import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { ChatBar } from "../chat/ChatBar";
import { ChatTranscript } from "../chat/ChatTranscript";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoChatContext } from "./types";

interface MakoChatViewProps {
  chat: MakoChatContext;
}

export function MakoChatView({ chat }: MakoChatViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [composerReserveHeight, setComposerReserveHeight] = useState(150);

  return (
    <View style={styles.container}>
      <ChatTranscript
        messages={chat.messages}
        sessionId={chat.sessionId}
        isStreaming={chat.isStreaming}
        isThinking={chat.isThinking}
        activeToolCallId={chat.activeToolCallId}
        onApproveTool={chat.onApproveTool}
        onDenyTool={chat.onDenyTool}
        onSubmitToolResult={chat.onSubmitToolResult}
        onPlanConfirm={chat.onPlanConfirm}
        bottomPadding={composerReserveHeight}
        showPlanTracker={false}
        emptyState={
          <View style={styles.emptyState}>
            <Text style={[styles.emptyTitle, { color: t.foreground }]}>
              Start a Mako chat
            </Text>
            <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
              Send a message to steer Mako, ask for status, or start a new run.
            </Text>
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

      <ChatBar
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
        researchEnabled={chat.researchEnabled}
        onResearchToggle={chat.onResearchToggle}
        tokenCount={chat.tokenCount}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
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
  errorBanner: {
    marginHorizontal: 16,
    marginBottom: 10,
    borderWidth: 1,
    borderRadius: 14,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  errorText: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
});
