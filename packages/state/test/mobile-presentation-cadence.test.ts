import {
  liveActivityStateEqual,
  shouldSyncChatWidget,
  type ChatWidgetCadenceState,
  type LiveActivitySemanticState,
} from "../../../apps/mobile/hooks/presentationCadence.ts";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const baseActivity: LiveActivitySemanticState = {
  chatTitle: "Build notifications",
  status: "working",
  toolCount: 2,
  filesAdded: 12,
  filesRemoved: 3,
};

Deno.test("Live Activity equality ignores render-only stream churn", () => {
  assert(
    liveActivityStateEqual(baseActivity, { ...baseActivity }),
    "equal semantic activity states should not trigger native updates",
  );
  assert(
    !liveActivityStateEqual(baseActivity, {
      ...baseActivity,
      status: "needs_input",
      toolApprovalId: "tool-1",
    }),
    "needs-input transitions must update immediately",
  );
});

const baseWidget: ChatWidgetCadenceState = {
  sessionId: "session-1",
  hasActiveSession: true,
  sessionTitle: "Build notifications",
  lastMessage: "Initial response",
  model: "gpt-5.4",
  isStreaming: true,
  tokenCount: 120,
  serverConnected: true,
};

Deno.test("chat widget skips token-by-token content while streaming", () => {
  assert(
    !shouldSyncChatWidget(baseWidget, {
      ...baseWidget,
      lastMessage: "Initial response with another streamed token",
      tokenCount: 121,
    }),
    "stream deltas should not reload WidgetKit timelines",
  );
});

Deno.test("chat widget syncs lifecycle transitions and settled content", () => {
  assert(
    shouldSyncChatWidget(baseWidget, {
      ...baseWidget,
      isStreaming: false,
      lastMessage: "Final response",
      tokenCount: 400,
    }),
    "completion must publish the final widget snapshot",
  );
  assert(
    shouldSyncChatWidget(baseWidget, {
      ...baseWidget,
      sessionId: "session-2",
    }),
    "session changes must publish a new widget snapshot",
  );
});
