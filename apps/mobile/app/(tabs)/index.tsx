import { useState, useRef, useCallback, useEffect } from "react";
import {
  AppState,
  View,
  FlatList,
  StyleSheet,
  Text,
  Pressable,
  Alert,
  Keyboard,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";
import { Menu, FileSearch } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import * as SecureStore from "../../platform/secure-store";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useSessionsStore, useStores } from "../../hooks/useStores";
import { MessageBubble } from "../../components/chat/MessageBubble";
import { KrustyLogo } from "../../components/ui/KrustyLogo";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ReportsViewer } from "../../components/ReportsViewer";
import { LinearGradient } from "../../platform/linear-gradient";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { useNotifications } from "../../hooks/useNotifications";
import Animated from "react-native-reanimated";
import type {
  ChatMessage,
  ContentBlock,
  MessageResponse,
  ModelInfo,
  SessionResponse,
  SessionStateResponse,
  SessionType,
  StreamCallbacks,
  ThinkingLevel,
  ToolCall,
} from "@krusty/api";

const TAB_TYPES: SessionType[] = ["chat", "code", "mako"];
const STATE_POLL_INTERVAL = 3_000;

function sessionTypeForTab(index: number): SessionType {
  return TAB_TYPES[index] ?? "code";
}

function tabForSessionType(type: SessionType): number {
  switch (type) {
    case "chat":
      return 0;
    case "mako":
      return 2;
    default:
      return 1;
  }
}

function isStreamingAgentState(agentState: string | null | undefined): boolean {
  return agentState === "streaming" || agentState === "tool_executing";
}

function stringifySessionBlockContent(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";

  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function buildRecoveryNotice(
  recovery: NonNullable<SessionStateResponse["recovery"]>,
): string {
  const headline =
    recovery.stop_reason === "stream_idle_timeout"
      ? "Previous turn stopped after the provider stream went idle."
      : recovery.stop_reason === "provider_error"
        ? "Previous turn stopped after a provider error."
        : recovery.stop_reason === "user_abort"
          ? "Previous turn was interrupted by user cancellation."
          : recovery.status === "tool_executing"
            ? "Previous turn ended while tool execution was in progress."
            : recovery.status === "streaming"
              ? "Previous turn ended while the assistant was still streaming."
              : "Previous turn ended before Krusty could safely finalize it.";

  const details: string[] = [];
  if (recovery.partial_assistant.text.trim()) {
    details.push(`Partial output: ${recovery.partial_assistant.text.trim()}`);
  }
  if (recovery.last_error?.trim()) {
    details.push(`Last error: ${recovery.last_error.trim()}`);
  }

  return details.length > 0 ? `${headline}\n\n${details.join("\n")}` : headline;
}

function applyRecoveryParity(
  messages: ChatMessage[],
  recovery: SessionStateResponse["recovery"],
  agentState: string,
): ChatMessage[] {
  let nextMessages = messages.filter(
    (message) => message.kind !== "recovery_notice",
  );

  if (recovery && agentState === "idle") {
    nextMessages = nextMessages.map((message) => ({
      ...message,
      toolCalls: message.toolCalls?.map((toolCall) => {
        if (
          (toolCall.status === "pending" || toolCall.status === "running") &&
          !toolCall.output
        ) {
          return {
            ...toolCall,
            status: "error" as const,
            output: "[Session interrupted - tool execution was cancelled]",
          };
        }
        return toolCall;
      }),
    }));

    nextMessages.unshift({
      role: "assistant",
      content: `[Recovery Notice] ${buildRecoveryNotice(recovery)}`,
      kind: "recovery_notice",
    });
  }

  return nextMessages;
}

function livePartialToolStatus(agentState: string): ToolCall["status"] {
  switch (agentState) {
    case "tool_executing":
      return "running";
    case "awaiting_input":
      return "awaiting_approval";
    default:
      return "pending";
  }
}

function applyLivePartialAssistant(
  messages: ChatMessage[],
  livePartial: SessionStateResponse["live_partial_assistant"],
  agentState: string,
): ChatMessage[] {
  const nextMessages = messages.filter(
    (message) => message.kind !== "live_partial",
  );
  if (
    !livePartial ||
    !["streaming", "tool_executing", "awaiting_input"].includes(agentState)
  ) {
    return nextMessages;
  }

  const hasContent = livePartial.text.trim().length > 0;
  const hasThinking = (livePartial.thinking?.trim().length ?? 0) > 0;
  const toolCalls = livePartial.tool_calls.map(
    (toolCall) =>
      ({
        id: toolCall.id,
        name: toolCall.name,
        status: livePartialToolStatus(agentState),
      }) satisfies ToolCall,
  );

  if (!hasContent && !hasThinking && toolCalls.length === 0) {
    return nextMessages;
  }

  return [
    ...nextMessages,
    {
      role: "assistant",
      content: livePartial.text,
      thinking: livePartial.thinking,
      toolCalls,
      kind: "live_partial",
    },
  ];
}

function applySessionStateOverlay(
  messages: ChatMessage[],
  serverState: SessionStateResponse | null,
): ChatMessage[] {
  if (!serverState) {
    return messages;
  }

  return applyLivePartialAssistant(
    applyRecoveryParity(
      messages,
      serverState.recovery,
      serverState.agent_state,
    ),
    serverState.live_partial_assistant,
    serverState.agent_state,
  );
}

function buildChatRequestContent(
  text: string,
  attachments: ChatBarAttachment[] = [],
): ContentBlock[] | undefined {
  if (attachments.length === 0) {
    return undefined;
  }

  const blocks: ContentBlock[] = [];
  for (const attachment of attachments) {
    if (attachment.type === "image" && attachment.base64) {
      blocks.push({
        type: "image",
        source: {
          type: "base64",
          media_type: attachment.mimeType ?? "image/jpeg",
          data: attachment.base64,
        },
      });
    }
  }

  const fileNames = attachments
    .filter((attachment) => attachment.type === "file")
    .map((attachment) => attachment.name);

  let textBlock = text.trim();
  if (fileNames.length > 0) {
    const label =
      fileNames.length === 1
        ? `[Attached file: ${fileNames[0]}]`
        : `[Attached files: ${fileNames.join(", ")}]`;
    textBlock = textBlock
      ? `${textBlock}

${label}`
      : label;
  }
  if (!textBlock && blocks.length > 0) {
    textBlock = "Please review the attached image.";
  }
  if (textBlock) {
    blocks.push({ type: "text", text: textBlock });
  }

  return blocks.length > 0 ? blocks : undefined;
}

function parseSessionMessages(messages: MessageResponse[]): ChatMessage[] {
  const toolResults = new Map<string, { output: string; isError: boolean }>();

  for (const message of messages) {
    for (const block of message.content as unknown as Array<
      Record<string, unknown>
    >) {
      if (!block || typeof block !== "object") continue;

      const toolUseId =
        typeof block.tool_use_id === "string" ? block.tool_use_id : null;

      if (block.type === "tool_result" && toolUseId) {
        toolResults.set(toolUseId, {
          output: stringifySessionBlockContent(
            block.output ?? block.content ?? "",
          ),
          isError: block.is_error === true,
        });
      }
    }
  }

  return messages
    .map((message) => {
      const role: ChatMessage["role"] =
        message.role === "user" ? "user" : "assistant";
      let content = "";
      let thinking = "";
      const toolCalls: ToolCall[] = [];

      for (const block of message.content as unknown as Array<
        Record<string, unknown>
      >) {
        if (!block || typeof block !== "object") continue;

        if (block.type === "text" && typeof block.text === "string") {
          content += content
            ? `
${block.text}`
            : block.text;
          continue;
        }

        if (block.type === "thinking" && typeof block.thinking === "string") {
          thinking += thinking
            ? `

${block.thinking}`
            : block.thinking;
          continue;
        }

        if (
          block.type === "tool_use" &&
          typeof block.id === "string" &&
          typeof block.name === "string"
        ) {
          const toolResult = toolResults.get(block.id);
          toolCalls.push({
            id: block.id,
            name: block.name,
            arguments:
              block.input && typeof block.input === "object"
                ? (block.input as Record<string, unknown>)
                : undefined,
            output: toolResult?.output,
            status: toolResult
              ? toolResult.isError
                ? "error"
                : "success"
              : "pending",
          });
        }
      }

      return {
        role,
        content,
        thinking: thinking || undefined,
        toolCalls,
      } satisfies ChatMessage;
    })
    .filter(
      (message) =>
        message.content.trim().length > 0 ||
        (message.thinking?.trim().length ?? 0) > 0 ||
        (message.toolCalls?.length ?? 0) > 0,
    );
}

export default function ChatScreen() {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);

  const sessions = useSessionsStore((s) => s.sessions) as SessionResponse[];
  const { sessions: sessionsStore } = useStores();
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessionTitle, setSessionTitle] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  const [isStreaming, setIsStreaming] = useState(false);
  const [isThinking, setIsThinking] = useState(false);
  const [model, setModel] = useState<string | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>("off");
  const [mode, setMode] = useState<"build" | "plan">("build");
  const [researchEnabled, setResearchEnabled] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [, setPendingApproval] = useState<{
    id: string;
    name: string;
    args: Record<string, unknown>;
  } | null>(null);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);

  function handleToolApprovalAction(id: string, approved: boolean) {
    void submitToolApprovalDecision(id, approved);
  }

  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: handleToolApprovalAction,
  });
  const { notifyToolApproval, notifyStreamComplete } = useNotifications({
    onToolApproval: handleToolApprovalAction,
  });

  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reportsOpen, setReportsOpen] = useState(false);

  const lastMsg = messages[messages.length - 1];
  useWidgetSync({
    hasActiveSession: !!sessionId,
    sessionTitle: sessionTitle || "Untitled",
    lastMessage:
      lastMsg?.role === "assistant" ? lastMsg.content?.slice(0, 200) || "" : "",
    model: model || "",
    isStreaming,
    tokenCount,
    serverConnected: isConnected,
  });

  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);
  const assistantRef = useRef<ChatMessage>({
    role: "assistant",
    content: "",
    toolCalls: [],
  });
  const flushTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const statePollingRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const t = theme.colors;

  const flushAssistantRef = useCallback(() => {
    const snapshot = {
      ...assistantRef.current,
      toolCalls: [...(assistantRef.current.toolCalls ?? [])],
    };
    setMessages((prev) => {
      const updated = [...prev];
      if (
        updated.length > 0 &&
        updated[updated.length - 1]?.role === "assistant"
      ) {
        updated[updated.length - 1] = snapshot;
      }
      return updated;
    });
  }, []);

  const startFlushTimer = useCallback(() => {
    if (flushTimerRef.current) return;
    flushTimerRef.current = setInterval(flushAssistantRef, 50);
  }, [flushAssistantRef]);

  const stopFlushTimer = useCallback(() => {
    if (flushTimerRef.current) {
      clearInterval(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    flushAssistantRef();
  }, [flushAssistantRef]);

  function clearLocalStreamState() {
    if (flushTimerRef.current) {
      clearInterval(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    abortRef.current = null;
    assistantRef.current = { role: "assistant", content: "", toolCalls: [] };
  }

  function stopStatePolling() {
    if (statePollingRef.current) {
      clearInterval(statePollingRef.current);
      statePollingRef.current = null;
    }
  }

  function updateToolCallState(
    toolCallId: string,
    updater: (toolCall: ToolCall) => ToolCall,
    flush = false,
  ) {
    const currentToolCalls = assistantRef.current.toolCalls ?? [];
    const currentIndex = currentToolCalls.findIndex(
      (toolCall) => toolCall.id === toolCallId,
    );
    if (currentIndex >= 0) {
      const nextToolCalls = [...currentToolCalls];
      nextToolCalls[currentIndex] = updater(nextToolCalls[currentIndex]!);
      assistantRef.current.toolCalls = nextToolCalls;
    }

    setMessages((prev) =>
      prev.map((message) => {
        if (
          !message.toolCalls?.some((toolCall) => toolCall.id === toolCallId)
        ) {
          return message;
        }
        return {
          ...message,
          toolCalls: message.toolCalls.map((toolCall) =>
            toolCall.id === toolCallId ? updater(toolCall) : toolCall,
          ),
        };
      }),
    );

    if (flush && currentIndex >= 0) {
      flushAssistantRef();
    }
  }

  function pushAssistantToolCall(toolCall: ToolCall) {
    assistantRef.current.toolCalls = [
      ...(assistantRef.current.toolCalls ?? []),
      toolCall,
    ];
    flushAssistantRef();
  }

  function applyLoadedSessionData(
    session: SessionResponse,
    rawMessages: MessageResponse[],
    serverState: SessionStateResponse | null,
  ) {
    setSessionId(session.id);
    setSessionTitle(session.title || "Untitled");
    setModel(session.model ?? null);
    setMode(serverState?.mode ?? session.mode ?? "build");
    setTokenCount(session.token_count ?? 0);
    setActiveTab(tabForSessionType(session.session_type));
    setMessages(
      applySessionStateOverlay(parseSessionMessages(rawMessages), serverState),
    );
    setIsStreaming(isStreamingAgentState(serverState?.agent_state));
    setIsThinking(
      serverState?.agent_state === "streaming" &&
        Boolean(serverState.live_partial_assistant?.thinking?.trim()),
    );
    setActiveToolCallId(null);
    setPendingApproval(null);
  }

  async function hydrateSessionFromServer(
    sessionIdToHydrate: string,
    source: "manual" | "poll" = "manual",
  ): Promise<boolean> {
    if (!client) return false;

    try {
      if (source === "manual") {
        clearLocalStreamState();
      }

      const [data, serverState] = await Promise.all([
        client.getSession(sessionIdToHydrate),
        client.getSessionState(sessionIdToHydrate).catch(() => null),
      ]);

      applyLoadedSessionData(data.session, data.messages, serverState);

      if (serverState && isStreamingAgentState(serverState.agent_state)) {
        if (source === "manual") {
          startStatePolling(sessionIdToHydrate);
        }
      } else {
        stopStatePolling();
      }

      return true;
    } catch {
      if (source === "manual") {
        setMessages([]);
        setIsStreaming(false);
        setIsThinking(false);
      }
      stopStatePolling();
      return false;
    }
  }

  function startStatePolling(sessionIdToPoll: string) {
    stopStatePolling();
    statePollingRef.current = setInterval(() => {
      void hydrateSessionFromServer(sessionIdToPoll, "poll");
    }, STATE_POLL_INTERVAL);
  }

  async function submitToolApprovalDecision(
    toolCallId: string,
    approved: boolean,
  ) {
    if (!client || !sessionId) return;

    setActiveToolCallId(toolCallId);
    updateToolCallState(
      toolCallId,
      (toolCall) =>
        approved
          ? { ...toolCall, status: "running", output: undefined }
          : {
              ...toolCall,
              status: "error",
              output: toolCall.output ?? "Denied by user",
            },
      true,
    );
    setPendingApproval(null);

    try {
      await client.submitToolApproval(sessionId, toolCallId, approved);
      setIsStreaming(true);
      setIsThinking(false);
      startStatePolling(sessionId);
    } catch {
      void hydrateSessionFromServer(sessionId, "manual");
    } finally {
      setActiveToolCallId(null);
    }
  }

  useEffect(() => {
    if (!client || !isConnected) return;

    sessionsStore.getState().loadSessions();
    client
      .getModels()
      .then(async (res) => {
        setModels(res.models);
        if (!model) {
          const saved = await SecureStore.getItemAsync("krusty_selected_model");
          const validSaved =
            saved && res.models.some((candidate) => candidate.id === saved);
          setModel(validSaved ? saved : res.default_model);
        }
      })
      .catch(() => {});
  }, [client, isConnected, model, sessionsStore]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState === "active" && sessionId && client) {
        void hydrateSessionFromServer(sessionId, "manual");
      }
    });

    return () => subscription.remove();
  }, [client, sessionId]);

  useEffect(() => {
    return () => {
      stopStatePolling();
      clearLocalStreamState();
    };
  }, []);

  async function loadSession(session: SessionResponse) {
    setDrawerOpen(false);
    await hydrateSessionFromServer(session.id, "manual");
  }

  async function handleNewSession() {
    if (!client) return;

    try {
      const session = await client.createSession(
        undefined,
        undefined,
        undefined,
        undefined,
        "chat",
      );
      sessionsStore.getState().loadSessions();
      stopStatePolling();
      clearLocalStreamState();
      setSessionId(session.id);
      setSessionTitle(session.title || "");
      setMessages([]);
      setMode("build");
      setTokenCount(0);
      setIsStreaming(false);
      setIsThinking(false);
      setActiveToolCallId(null);
      setPendingApproval(null);
      setDrawerOpen(false);
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch {
      // silent
    }
  }

  async function handleDirectorySelected(path: string) {
    if (!client) return;

    try {
      const type = sessionTypeForTab(activeTab);
      const session = await client.createSession(
        undefined,
        path,
        undefined,
        "selected",
        type,
      );
      sessionsStore.getState().loadSessions();
      stopStatePolling();
      clearLocalStreamState();
      setSessionId(session.id);
      setSessionTitle(session.title || "");
      setMessages([]);
      setMode("build");
      setTokenCount(0);
      setIsStreaming(false);
      setIsThinking(false);
      setActiveToolCallId(null);
      setPendingApproval(null);
      setDrawerOpen(false);
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch {
      // silent
    }
  }

  function handleDeleteSession(id: string) {
    if (!client) return;

    Alert.alert("Delete Session", "Delete this session?", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: async () => {
          try {
            await client.deleteSession(id);
            sessionsStore.getState().loadSessions();
            if (sessionId === id) {
              stopStatePolling();
              clearLocalStreamState();
              setSessionId(null);
              setSessionTitle("");
              setMessages([]);
              setMode("build");
              setTokenCount(0);
              setIsStreaming(false);
              setIsThinking(false);
              setActiveToolCallId(null);
              setPendingApproval(null);
            }
          } catch {
            // silent
          }
        },
      },
    ]);
  }

  const CHAT_BAR_ZONE = 130;
  const msgLen = messages.length;
  const lastMsgContent = messages[msgLen - 1]?.content?.length ?? 0;

  const scrollToBottom = useCallback(() => {
    const content = contentHeightRef.current;
    const viewport = listHeightRef.current;
    if (!content || !viewport || content <= viewport) return;
    const maxOffset = content - viewport;
    const targetOffset = Math.max(0, maxOffset - CHAT_BAR_ZONE);
    flatListRef.current?.scrollToOffset({
      offset: targetOffset,
      animated: !isStreaming,
    });
  }, [isStreaming]);

  useEffect(() => {
    if (msgLen > 0) {
      requestAnimationFrame(scrollToBottom);
    }
  }, [msgLen, lastMsgContent, scrollToBottom]);

  async function runAssistantStream(
    currentSessionId: string,
    executeStream: (
      callbacks: StreamCallbacks,
      signal: AbortSignal,
    ) => Promise<void>,
  ): Promise<boolean> {
    const streamStartedAt = Date.now();
    let latestChatTitle = sessionTitle || "Chat";
    let latestTokenCount = tokenCount;
    let streamFailed = false;

    startActivity(sessionTitle || "Chat", model || "unknown");
    setIsStreaming(true);
    setIsThinking(false);

    const abort = new AbortController();
    abortRef.current = abort;
    startFlushTimer();

    const callbacks: StreamCallbacks = {
      onTextDelta: (delta) => {
        assistantRef.current.content += delta;
        setIsThinking(false);
        updateActivity({
          status: "streaming",
          currentText: assistantRef.current.content.slice(-200),
        });
      },
      onThinkingDelta: (thinking) => {
        assistantRef.current.thinking =
          (assistantRef.current.thinking ?? "") + thinking;
        setIsThinking(true);
        updateActivity({ status: "thinking", currentText: "Thinking..." });
      },
      onToolCallStart: (id, name) => {
        setIsThinking(false);
        pushAssistantToolCall({ id, name, status: "running" });
        updateActivity({ status: "tool_call", currentTool: name });
      },
      onToolCallComplete: (id, _name, args) => {
        updateToolCallState(id, (toolCall) => ({
          ...toolCall,
          arguments: args,
        }));
      },
      onToolResult: (id, output, isError) => {
        updateToolCallState(
          id,
          (toolCall) => ({
            ...toolCall,
            output,
            status: isError ? "error" : "success",
          }),
          true,
        );
      },
      onToolOutputDelta: (id, delta) => {
        updateToolCallState(id, (toolCall) => ({
          ...toolCall,
          output: (toolCall.output ?? "") + delta,
        }));
      },
      onDelegatedProgress: () => {},
      onToolApprovalRequired: (id, name, args) => {
        updateToolCallState(
          id,
          (toolCall) => ({
            ...toolCall,
            arguments: args,
            status: "awaiting_approval",
          }),
          true,
        );
        setPendingApproval({ id, name, args });
        updateActivity({
          status: "awaiting_approval",
          toolApprovalId: id,
          toolApprovalName: name,
        });
        notifyToolApproval(id, name, currentSessionId);
        Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
        Alert.alert("Tool Approval", `Allow "${name}" to execute?`, [
          {
            text: "Deny",
            style: "destructive",
            onPress: () => {
              void submitToolApprovalDecision(id, false);
            },
          },
          {
            text: "Allow",
            onPress: () => {
              void submitToolApprovalDecision(id, true);
            },
          },
        ]);
      },
      onToolApproved: (id) => {
        updateToolCallState(
          id,
          (toolCall) => ({ ...toolCall, status: "running", output: undefined }),
          true,
        );
        setPendingApproval(null);
      },
      onToolDenied: (id) => {
        updateToolCallState(
          id,
          (toolCall) => ({
            ...toolCall,
            status: "error",
            output: toolCall.output ?? "Denied by user",
          }),
          true,
        );
        setPendingApproval(null);
      },
      onTurnComplete: (_turn, hasMore) => {
        if (hasMore) {
          flushAssistantRef();
          assistantRef.current = {
            role: "assistant",
            content: "",
            toolCalls: [],
          };
          setMessages((prev) => [...prev, assistantRef.current]);
        }
      },
      onPlanUpdate: () => {},
      onModeChange: (nextMode) => {
        if (nextMode === "build" || nextMode === "plan") {
          setMode(nextMode);
        }
      },
      onPlanComplete: (toolCallId, title, taskCount) => {
        pushAssistantToolCall({
          id: toolCallId,
          name: "PlanConfirm",
          arguments: { title, task_count: taskCount },
          status: "pending",
        });
      },
      onUsage: (promptTokens, completionTokens) => {
        const total = promptTokens + completionTokens;
        latestTokenCount = total;
        setTokenCount(total);
        updateActivity({ tokenCount: total });
      },
      onTitleUpdate: (title) => {
        latestChatTitle = title;
        setSessionTitle(title);
        sessionsStore.getState().loadSessions();
        updateActivity({ chatTitle: title });
      },
      onFinish: (finishedSessionId) => {
        stopFlushTimer();
        abortRef.current = null;
        setSessionId(finishedSessionId);
        setIsStreaming(false);
        setIsThinking(false);
        setPendingApproval(null);
        endActivity();
        const elapsedSeconds = Math.floor(
          (Date.now() - streamStartedAt) / 1000,
        );
        notifyStreamComplete(
          finishedSessionId,
          latestChatTitle,
          latestTokenCount,
          elapsedSeconds,
        );
      },
      onError: () => {
        streamFailed = true;
        stopFlushTimer();
        abortRef.current = null;
        setIsStreaming(false);
        setIsThinking(false);
        setPendingApproval(null);
        endActivity();
      },
    };

    try {
      await executeStream(callbacks, abort.signal);
    } catch {
      streamFailed = true;
      stopFlushTimer();
      abortRef.current = null;
      setIsStreaming(false);
      setIsThinking(false);
      setPendingApproval(null);
      endActivity();
    }

    return !streamFailed;
  }

  async function handleInteractiveToolResult(
    toolCallId: string,
    result: string,
  ) {
    if (!client || !sessionId || activeToolCallId) return;

    setActiveToolCallId(toolCallId);
    updateToolCallState(
      toolCallId,
      (toolCall) => ({ ...toolCall, output: result, status: "success" }),
      true,
    );

    assistantRef.current = { role: "assistant", content: "", toolCalls: [] };
    setMessages((prev) => [...prev, assistantRef.current]);

    const success = await runAssistantStream(sessionId, (callbacks, signal) =>
      client.streamToolResult(
        {
          session_id: sessionId,
          tool_call_id: toolCallId,
          result,
        },
        callbacks,
        signal,
      ),
    );

    setActiveToolCallId(null);
    if (!success) {
      await hydrateSessionFromServer(sessionId, "manual");
    }
  }

  async function handlePlanConfirm(
    toolCallId: string,
    choice: "execute" | "abandon",
  ) {
    if (choice === "execute") {
      setMode("build");
    }
    await handleInteractiveToolResult(toolCallId, JSON.stringify({ choice }));
  }

  async function handleSend(
    content: string,
    attachments: ChatBarAttachment[] = [],
  ) {
    const trimmed = content.trim();
    if (!client || (!trimmed && attachments.length === 0)) return;

    assistantRef.current = { role: "assistant", content: "", toolCalls: [] };
    const attachmentLabel =
      attachments.length > 0
        ? `[Attachments: ${attachments.map((attachment) => attachment.name).join(", ")}]`
        : "";
    const displayContent = trimmed
      ? attachmentLabel
        ? `${trimmed}\n\n${attachmentLabel}`
        : trimmed
      : attachmentLabel || "Attached content";

    setMessages((prev) => [
      ...prev,
      { role: "user", content: displayContent, toolCalls: [] },
      assistantRef.current,
    ]);

    let currentSessionId = sessionId;
    if (!currentSessionId) {
      try {
        const session = await client.createSession(
          undefined,
          undefined,
          undefined,
          undefined,
          sessionTypeForTab(activeTab),
        );
        currentSessionId = session.id;
        setSessionId(session.id);
        setSessionTitle(session.title || "");
        sessionsStore.getState().loadSessions();
      } catch {
        setMessages((prev) => prev.slice(0, -2));
        return;
      }
    }

    const success = await runAssistantStream(
      currentSessionId,
      (callbacks, signal) =>
        client.streamChat(
          {
            session_id: currentSessionId!,
            message: trimmed || "Please review the attached content.",
            content: buildChatRequestContent(trimmed, attachments),
            model: model ?? undefined,
            thinking_enabled:
              thinkingLevel !== "off" ? thinkingLevel : undefined,
            mode,
          },
          callbacks,
          signal,
        ),
    );

    if (!success && currentSessionId) {
      await hydrateSessionFromServer(currentSessionId, "manual");
    }
  }

  function handleStop() {
    abortRef.current?.abort();
    abortRef.current = null;
    stopFlushTimer();
    setIsStreaming(false);
    setIsThinking(false);
    setPendingApproval(null);
    endActivity();
  }

  function handleModelSelect(modelId: string) {
    setModel(modelId);
    SecureStore.setItemAsync("krusty_selected_model", modelId).catch(() => {});
  }

  function handleTabChange(index: number) {
    setActiveTab(index);
    const currentSession = sessions.find((session) => session.id === sessionId);
    if (
      currentSession &&
      currentSession.session_type !== sessionTypeForTab(index)
    ) {
      stopStatePolling();
      clearLocalStreamState();
      setSessionId(null);
      setSessionTitle("");
      setMessages([]);
      setMode("build");
      setTokenCount(0);
      setIsStreaming(false);
      setIsThinking(false);
      setActiveToolCallId(null);
      setPendingApproval(null);
    }
  }

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {/* Top bar */}
      <Animated.View style={[styles.topBar, entrance.topBarStyle]}>
        {!isDesktop && (
          <Pressable
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setDrawerOpen(true);
            }}
            style={styles.menuBtn}
          >
            <Menu size={22} color={t.foreground} strokeWidth={1.8} />
          </Pressable>
        )}

        <Pressable
          onPress={() => {
            if (!sessionId || !client || !sessionTitle) return;
            Alert.prompt(
              "Rename Session",
              undefined,
              async (newTitle: string) => {
                if (newTitle && newTitle.trim()) {
                  setSessionTitle(newTitle.trim());
                  await client.updateSession(sessionId, {
                    title: newTitle.trim(),
                  });
                  sessionsStore.getState().loadSessions();
                }
              },
              "plain-text",
              sessionTitle,
            );
          }}
          style={styles.titleBtn}
          disabled={!sessionTitle}
        >
          <Text
            style={[
              styles.title,
              { color: sessionTitle ? t.foreground : "transparent" },
            ]}
            numberOfLines={1}
          >
            {sessionTitle || " "}
          </Text>
        </Pressable>

        <Pressable
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setReportsOpen(true);
          }}
          style={styles.menuBtn}
        >
          <FileSearch size={20} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      </Animated.View>

      {/* Messages */}
      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        {messages.length === 0 ? (
          <Pressable style={styles.empty} onPress={Keyboard.dismiss}>
            <KrustyLogo />
          </Pressable>
        ) : (
          <View style={styles.flex}>
            <FlatList
              ref={flatListRef}
              data={messages}
              keyExtractor={(_, i) => String(i)}
              onScrollBeginDrag={Keyboard.dismiss}
              renderItem={({
                item,
                index,
              }: {
                item: ChatMessage;
                index: number;
              }) => (
                <MessageBubble
                  message={item}
                  isLast={index === messages.length - 1}
                  isStreaming={isStreaming && index === messages.length - 1}
                  isThinking={isThinking && index === messages.length - 1}
                  activeToolCallId={activeToolCallId}
                  onApproveTool={(toolCallId) =>
                    handleToolApprovalAction(toolCallId, true)
                  }
                  onDenyTool={(toolCallId) =>
                    handleToolApprovalAction(toolCallId, false)
                  }
                  onSubmitToolResult={handleInteractiveToolResult}
                  onPlanConfirm={handlePlanConfirm}
                />
              )}
              style={styles.flex}
              contentContainerStyle={[
                styles.list,
                isDesktop && styles.listDesktop,
              ]}
              onLayout={(e) => {
                listHeightRef.current = e.nativeEvent.layout.height;
              }}
              onContentSizeChange={(_w, h) => {
                contentHeightRef.current = h;
              }}
              keyboardDismissMode="interactive"
              keyboardShouldPersistTaps="handled"
            />
            {/* Fade edges */}
            <LinearGradient
              colors={[t.background, t.background + "00"]}
              style={styles.fadeTop}
              pointerEvents="none"
            />
            <LinearGradient
              colors={[t.background + "00", t.background]}
              style={styles.fadeBottom}
              pointerEvents="none"
            />
          </View>
        )}
      </Animated.View>

      {/* Chat bar */}
      <Animated.View style={[entrance.bottomBarStyle, { overflow: "visible" }]}>
        <ChatBar
          onSend={handleSend}
          onStop={handleStop}
          isStreaming={isStreaming}
          disabled={!isConnected}
          thinkingLevel={thinkingLevel}
          onThinkingChange={setThinkingLevel}
          mode={mode}
          onModeToggle={() =>
            setMode((m) => (m === "build" ? "plan" : "build"))
          }
          onModelSelect={handleModelSelect}
          model={model}
          models={models}
          sessionType={sessionTypeForTab(activeTab)}
          researchEnabled={researchEnabled}
          onResearchToggle={() => setResearchEnabled((r) => !r)}
          tokenCount={tokenCount}
        />
      </Animated.View>

      {/* Reports viewer */}
      <ReportsViewer
        visible={reportsOpen}
        onClose={() => setReportsOpen(false)}
      />
    </SafeAreaView>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={loadSession}
      onNewSession={handleNewSession}
      onNewSessionWithDir={handleDirectorySelected}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.push("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={handleTabChange}
    >
      {chatContent}

      {/* Session drawer — mobile only */}
      {!isDesktop && (
        <SessionDrawer
          isOpen={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={loadSession}
          onNewSession={handleNewSession}
          onNewSessionWithDir={handleDirectorySelected}
          onDeleteSession={handleDeleteSession}
          onOpenSettings={() => {
            setDrawerOpen(false);
            router.push("/(tabs)/settings");
          }}
          activeTab={activeTab}
          onTabChange={handleTabChange}
        />
      )}
    </DesktopShell>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  menuBtn: {
    padding: 4,
  },
  titleBtn: {
    flex: 1,
  },
  title: {
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  list: {
    paddingHorizontal: 16,
    paddingTop: 8,
    paddingBottom: 16,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: "center",
    width: "100%",
  },
  fadeTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 64,
  },
  fadeBottom: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: 120,
  },
  empty: {
    flex: 1,
    justifyContent: "flex-start",
    alignItems: "center",
    paddingTop: "35%",
    gap: 16,
  },
  emptyTitle: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  emptyHint: {
    fontSize: 17,
  },
  stubTitle: {
    fontSize: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  stubText: {
    fontSize: 15,
    marginTop: 8,
  },
  modalBackdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0,0,0,0.6)",
    justifyContent: "flex-end",
    zIndex: 200,
  },
  modelPicker: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: "60%",
    paddingTop: 20,
    paddingBottom: 40,
    backgroundColor: "#1a1f2e",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: "rgba(255,255,255,0.1)",
  },
  modelPickerTitle: {
    fontSize: 18,
    fontWeight: "700",
    textAlign: "center",
    marginBottom: 16,
  },
  modelList: {
    paddingHorizontal: 16,
  },
  modelItem: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 8,
  },
  modelName: {
    fontSize: 16,
    fontWeight: "500",
  },
  modelProvider: {
    fontSize: 13,
    marginTop: 2,
  },
});
