import { useEffect, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import type { ChatMessage, SessionType } from "@krusty/api";
import { shallow } from "zustand/shallow";

import { useStores } from "../../hooks/useStores";
import { useThemeContext } from "../../hooks/useTheme";
import { KrustyLogo } from "../ui/KrustyLogo";
import { ChatTranscript } from "./ChatTranscript";

export interface ModeConversationSurfaceProps {
  mode: SessionType;
  active: boolean;
  externalBottomPadding: number;
  hideJumpToLatest?: boolean;
  activeToolCallId?: string | null;
  onApproveTool?: (sessionId: string, toolCallId: string) => void;
  onDenyTool?: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult?: (
    toolCallId: string,
    result: string,
  ) => void | Promise<void>;
  onPlanConfirm?: (
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void | Promise<void>;
  emptyError?: string | null;
}

interface ModeView {
  sessionId: string | null;
  messages: ChatMessage[];
  isStreaming: boolean;
  isThinking: boolean;
  isLoading: boolean;
  error: string | null;
}

function readModeView(
  getState: () => {
    sessionId: string | null;
    messages: ChatMessage[];
    isStreaming: boolean;
    isThinking: boolean;
    isLoading: boolean;
    error: string | null;
  },
): ModeView {
  const state = getState();
  return {
    sessionId: state.sessionId ?? null,
    messages: state.messages ?? [],
    isStreaming: state.isStreaming ?? false,
    isThinking: state.isThinking ?? false,
    isLoading: state.isLoading ?? false,
    error: state.error ?? null,
  };
}

/**
 * Keep each mode's transcript tree mounted so mode swipes do not pay a full
 * unmount/remount tax. Inactive modes freeze their last painted snapshot and
 * unsubscribe from stream ticks so background modes cannot thrash JS.
 */
export function ModeConversationSurface({
  mode,
  active,
  externalBottomPadding,
  hideJumpToLatest = false,
  activeToolCallId,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
  emptyError,
}: ModeConversationSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const stores = useStores();
  const sessionStore = stores?.modes[mode].session;

  const [view, setView] = useState<ModeView>(() =>
    sessionStore
      ? readModeView(sessionStore.getState)
      : {
          sessionId: null,
          messages: [],
          isStreaming: false,
          isThinking: false,
          isLoading: false,
          error: null,
        },
  );
  const frozenRef = useRef<ModeView>(view);

  useEffect(() => {
    if (!sessionStore) {
      return;
    }

    // Always paint the latest shell when this mode becomes active.
    if (active) {
      const next = readModeView(sessionStore.getState);
      frozenRef.current = next;
      setView((current) => (shallow(current, next) ? current : next));
    }

    // Inactive modes keep their last painted snapshot and ignore stream ticks.
    if (!active) {
      return;
    }

    return sessionStore.subscribe((state) => {
      const next = {
        sessionId: state.sessionId ?? null,
        messages: state.messages ?? [],
        isStreaming: state.isStreaming ?? false,
        isThinking: state.isThinking ?? false,
        isLoading: state.isLoading ?? false,
        error: state.error ?? null,
      };
      frozenRef.current = next;
      setView((current) => (shallow(current, next) ? current : next));
    });
  }, [active, sessionStore]);

  const painted = active ? view : frozenRef.current;
  const showError = emptyError ?? painted.error;

  return (
    <View
      style={[
        styles.host,
        active ? styles.active : styles.inactive,
      ]}
      pointerEvents={active ? "auto" : "none"}
      accessibilityElementsHidden={!active}
      importantForAccessibility={active ? "auto" : "no-hide-descendants"}
    >
      <ChatTranscript
        messages={painted.messages}
        sessionId={painted.sessionId}
        sessionType={mode}
        scrollStateKey={`${mode}:${painted.sessionId ?? "new"}`}
        isStreaming={painted.isStreaming}
        isThinking={painted.isThinking}
        isLoading={painted.isLoading}
        activeToolCallId={active ? activeToolCallId : null}
        onApproveTool={onApproveTool}
        onDenyTool={onDenyTool}
        onSubmitToolResult={onSubmitToolResult}
        onPlanConfirm={onPlanConfirm}
        bottomPadding={externalBottomPadding}
        hideJumpToLatest={hideJumpToLatest || !active}
        isActive={active}
        showPlanTracker={mode !== "mako"}
        emptyState={
          <View style={styles.empty}>
            <KrustyLogo />
            {showError ? (
              <Text style={[styles.emptyHint, { color: t.error }]}>
                {showError}
              </Text>
            ) : null}
          </View>
        }
      />
    </View>
  );
}

const styles = StyleSheet.create({
  host: {
    ...StyleSheet.absoluteFillObject,
  },
  active: {
    opacity: 1,
    zIndex: 2,
  },
  inactive: {
    opacity: 0,
    zIndex: 0,
  },
  empty: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  emptyHint: {
    marginTop: 12,
    fontSize: 13,
    lineHeight: 18,
    textAlign: "center",
    paddingHorizontal: 24,
  },
});
