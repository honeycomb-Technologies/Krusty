import type {
  PartialAssistantState as ApiPartialAssistantState,
  SessionRecoveryState as ApiRecoveryState,
} from '@krusty/api';

import {
  createDelegatedArtifactState,
  resolveDelegatedKind,
} from './delegated';
import type { ChatMessage, ToolCall } from './types';

export function toErrorMessage(err: unknown, fallback = 'Unknown error'): string {
  return err instanceof Error ? err.message : fallback;
}

function generateRuntimeId(prefix: string): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (typeof randomUuid === 'string' && randomUuid.length > 0) {
    return `${prefix}-${randomUuid}`;
  }

  return [
    prefix,
    Date.now().toString(36),
    Math.random().toString(36).slice(2, 10),
  ].join('-');
}

export function createChatMessageId(prefix: string): string {
  return generateRuntimeId(prefix);
}

export function buildStoredMessageId(index: number, message: ChatMessage): string {
  const firstToolId = message.toolCalls?.[0]?.id;
  if (firstToolId) {
    return `stored-${index}-${message.role}-${firstToolId}`;
  }

  return [
    'stored',
    index,
    message.role,
    message.content.length,
    message.thinking?.length ?? 0,
  ].join('-');
}

function isTransientAssistantKind(
  kind: ChatMessage['kind'] | undefined,
): kind is 'live_partial' | 'streaming' {
  return kind === 'live_partial' || kind === 'streaming';
}

function isTransientAssistantMessage(
  message: ChatMessage | undefined,
): boolean {
  return message?.role === 'assistant' && isTransientAssistantKind(message.kind);
}

export function upsertTransientAssistantMessage(
  messages: ChatMessage[],
  message: ChatMessage,
): ChatMessage[] {
  const nextMessages = messages.filter((entry) => entry.kind !== 'live_partial');
  const lastIndex = nextMessages.length - 1;
  const lastMessage = nextMessages[lastIndex];

  if (lastIndex >= 0 && isTransientAssistantMessage(lastMessage)) {
    nextMessages[lastIndex] = { ...message, id: lastMessage.id };
    return nextMessages;
  }

  return [...nextMessages, message];
}

export function finalizeTransientAssistantMessages(
  messages: ChatMessage[],
): ChatMessage[] {
  return messages.map((message) =>
    isTransientAssistantMessage(message)
      ? { ...message, kind: undefined }
      : message,
  );
}

export function pruneEmptyAssistantMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.filter((message) => {
    if (message.role !== 'assistant') {
      return true;
    }

    return (
      message.content.trim().length > 0
      || (message.thinking?.trim().length ?? 0) > 0
      || (message.toolCalls?.length ?? 0) > 0
    );
  });
}

function buildRecoveryNotice(recovery: ApiRecoveryState): string {
  const headline =
    recovery.stop_reason === 'stream_idle_timeout'
      ? 'Previous turn stopped after the provider stream went idle.'
      : recovery.stop_reason === 'provider_error'
        ? 'Previous turn stopped after a provider error.'
        : recovery.stop_reason === 'user_abort'
          ? 'Previous turn was interrupted by user cancellation.'
          : recovery.status === 'tool_executing'
            ? 'Previous turn ended while tool execution was in progress.'
            : recovery.status === 'streaming'
              ? 'Previous turn ended while the assistant was still streaming.'
              : 'Previous turn ended before Krusty could safely finalize it.';

  const details: string[] = [];
  if (recovery.partial_assistant.text.trim()) {
    details.push(`Partial output: ${recovery.partial_assistant.text.trim()}`);
  }
  if (recovery.last_error?.trim()) {
    details.push(`Last error: ${recovery.last_error.trim()}`);
  }

  return details.length > 0 ? `${headline}

${details.join('
')}` : headline;
}

export function applyRecoveryParity(
  messages: ChatMessage[],
  recovery: ApiRecoveryState | null | undefined,
  agentState: string,
): ChatMessage[] {
  let nextMessages = messages.filter((message) => message.kind !== 'recovery_notice');

  if (recovery && agentState === 'idle') {
    nextMessages = nextMessages.map((message) => ({
      ...message,
      toolCalls: message.toolCalls?.map((toolCall) => {
        if (
          (toolCall.status === 'pending' || toolCall.status === 'running')
          && !toolCall.output
        ) {
          return {
            ...toolCall,
            status: 'error' as const,
            output: '[Session interrupted - tool execution was cancelled]',
          };
        }
        return toolCall;
      }),
    }));

    nextMessages.unshift({
      id: [
        'recovery-notice',
        recovery.schema_version,
        recovery.status,
        recovery.stop_reason ?? 'unknown',
      ].join('-'),
      role: 'assistant',
      content: `[Recovery Notice] ${buildRecoveryNotice(recovery)}`,
      kind: 'recovery_notice',
    });
  }

  return nextMessages;
}

function livePartialToolStatus(agentState: string): ToolCall['status'] {
  switch (agentState) {
    case 'tool_executing':
      return 'running';
    case 'awaiting_input':
      return 'awaiting_approval';
    default:
      return 'pending';
  }
}

export function applyLivePartialAssistant(
  messages: ChatMessage[],
  livePartial: ApiPartialAssistantState | null | undefined,
  agentState: string,
): ChatMessage[] {
  const nextMessages = messages.filter((message) => message.kind !== 'live_partial');
  if (
    !livePartial
    || !['streaming', 'tool_executing', 'awaiting_input'].includes(agentState)
  ) {
    return nextMessages;
  }

  const hasContent = livePartial.text.trim().length > 0;
  const hasThinking = (livePartial.thinking?.trim().length ?? 0) > 0;
  const toolCalls = livePartial.tool_calls.map(
    (toolCall) => {
      const delegatedKind = resolveDelegatedKind(toolCall.name);
      return {
        id: toolCall.id,
        name: toolCall.name,
        delegated: delegatedKind
          ? createDelegatedArtifactState(delegatedKind)
          : undefined,
        status: livePartialToolStatus(agentState),
      } satisfies ToolCall;
    },
  );

  if (!hasContent && !hasThinking && toolCalls.length === 0) {
    return nextMessages;
  }

  return upsertTransientAssistantMessage(nextMessages, {
    id: createChatMessageId('live-partial'),
    role: 'assistant',
    content: livePartial.text,
    thinking: livePartial.thinking,
    toolCalls,
    kind: 'live_partial',
  });
}
