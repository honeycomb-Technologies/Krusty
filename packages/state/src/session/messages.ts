import type { ContentBlock } from '@krusty/api';

import {
  MAX_MESSAGE_CONTENT_LENGTH,
  SUPPORTED_IMAGE_MIME_TYPES,
} from './constants';
import { formatToolOutputForDisplay, parseDelegatedArtifactState } from './delegated';
import { buildStoredMessageId } from './transient';
import type { Attachment, ChatMessage, ToolCall } from './types';

function extractTextContent(content: unknown): string {
  if (typeof content === 'string') return content;

  if (Array.isArray(content)) {
    let text = '';
    for (const block of content) {
      if (!block || typeof block !== 'object') continue;
      if (block.type !== 'text' || typeof block.text !== 'string') continue;
      text += text ? `\n${block.text}` : block.text;
    }
    return text;
  }

  if (content && typeof content === 'object' && 'text' in content) {
    const textValue = (content as Record<string, unknown>).text;
    if (typeof textValue === 'string') return textValue;
  }

  return '';
}

function parseStoredMessage(
  message: { role: string; content: unknown },
  toolResults?: Map<string, { output: string; isError: boolean }>,
): ChatMessage {
  const role: 'user' | 'assistant' =
    message.role === 'user' || message.role === 'assistant'
      ? message.role
      : 'assistant';
  const parsed: ChatMessage = {
    id: '',
    role,
    content: '',
    thinking: '',
    toolCalls: [],
  };
  const contentArray = Array.isArray(message.content) ? message.content : [];

  for (const block of contentArray) {
    if (!block || typeof block !== 'object') continue;

    if (block.type === 'text' || ('text' in block && !block.type)) {
      if (parsed.content.length < MAX_MESSAGE_CONTENT_LENGTH) {
        parsed.content += (parsed.content ? '\n' : '') + (block.text || '');
      }
    } else if (block.type === 'thinking' || 'thinking' in block) {
      const thinkingContent = block.thinking || '';
      parsed.thinking = parsed.thinking
        ? `${parsed.thinking}\n\n${thinkingContent}`
        : thinkingContent;
    } else if (
      block.type === 'tool_use'
      || ('id' in block && 'name' in block && 'input' in block)
    ) {
      parsed.toolCalls = parsed.toolCalls || [];
      const toolResult = toolResults?.get(block.id);
      const delegated = parseDelegatedArtifactState(
        block.name,
        toolResult?.output,
        block.input,
      );
      const status: ToolCall['status'] = toolResult
        ? delegated?.outcome === 'partial'
          ? 'partial'
          : delegated?.outcome === 'failed'
            ? 'error'
            : toolResult.isError
              ? 'error'
              : 'success'
        : 'pending';
      parsed.toolCalls.push({
        id: block.id,
        name: block.name,
        arguments: block.input,
        output: formatToolOutputForDisplay(
          block.name,
          toolResult?.output,
          block.input,
        ),
        delegatedRunId: delegated?.delegatedRunId,
        delegated,
        status,
      });
    }
  }

  if (
    !parsed.content
    && !parsed.thinking
    && (!parsed.toolCalls || parsed.toolCalls.length === 0)
  ) {
    parsed.content = extractTextContent(message.content);
  }

  return parsed;
}

export function processStoredMessages(
  rawMessages: { role: string; content: unknown }[],
): ChatMessage[] {
  const result: ChatMessage[] = [];
  const toolResults = new Map<string, { output: string; isError: boolean }>();

  for (const message of rawMessages) {
    const contentArray = Array.isArray(message.content) ? message.content : [];
    for (const block of contentArray) {
      if (!block || typeof block !== 'object') continue;
      if (block.type === 'tool_result' || 'tool_use_id' in block) {
        if (block.tool_use_id) {
          const output =
            typeof block.output === 'string'
              ? block.output
              : typeof block.content === 'string'
                ? block.content
                : JSON.stringify(block.output || block.content || '');
          toolResults.set(block.tool_use_id, {
            output,
            isError: block.is_error === true,
          });
        }
      }
    }
  }

  for (const message of rawMessages) {
    const parsed = parseStoredMessage(message, toolResults);
    const hasContent = parsed.content.trim().length > 0;
    const hasThinking = (parsed.thinking?.trim().length ?? 0) > 0;
    const hasToolCalls = (parsed.toolCalls?.length ?? 0) > 0;
    if (hasContent || hasThinking || hasToolCalls) {
      parsed.id = buildStoredMessageId(result.length, parsed);
      result.push(parsed);
    }
  }

  return result;
}

export function buildContentBlocks(
  text: string,
  attachments: Attachment[],
): ContentBlock[] {
  const blocks: ContentBlock[] = [];

  for (const attachment of attachments) {
    if (attachment.type === 'image' && attachment.base64) {
      const mediaType = normalizeSupportedImageMimeType(attachment.mimeType);
      if (!mediaType) {
        throw new Error(unsupportedImageMimeTypeMessage(attachment.mimeType));
      }
      blocks.push({
        type: 'image',
        source: {
          type: 'base64',
          media_type: mediaType,
          data: attachment.base64,
        },
      });
    }
  }

  const fileSections: string[] = [];
  const attachedFileNames: string[] = [];
  for (const attachment of attachments) {
    if (attachment.type !== 'file') continue;
    attachedFileNames.push(attachment.name);
    if (attachment.text) {
      fileSections.push(`\n\n--- ${attachment.name} ---\n${attachment.text}`);
    }
  }

  const fileContent = fileSections.join('');
  const fallbackFileLabel =
    attachedFileNames.length > 0 && fileContent.length === 0
      ? `\n\n[Attached files: ${attachedFileNames.join(', ')}]`
      : '';
  const fullText = `${text}${fileContent}${fallbackFileLabel}`.trim();
  blocks.push({
    type: 'text',
    text: fullText.length > 0 ? fullText : 'Attached content',
  });

  return blocks;
}

function normalizeSupportedImageMimeType(
  mimeType: string | null | undefined,
): string | null {
  const normalized = (mimeType || 'image/png').trim().toLowerCase();
  switch (normalized) {
    case 'image/jpeg':
    case 'image/jpg':
    case 'image/pjpeg':
      return 'image/jpeg';
    case 'image/png':
      return 'image/png';
    case 'image/gif':
      return 'image/gif';
    case 'image/webp':
      return 'image/webp';
    default:
      return null;
  }
}

export function unsupportedImageMimeTypeMessage(mimeType: string): string {
  const normalized = mimeType.trim().toLowerCase();
  const hint =
    normalized === 'image/heic' || normalized === 'image/heif'
      ? ' Convert HEIC/HEIF images to JPEG or PNG before uploading.'
      : '';
  return `Image format '${mimeType.trim()}' is not supported. Supported formats: ${SUPPORTED_IMAGE_MIME_TYPES.join(', ')}.${hint}`;
}

export function getUnsupportedImageAttachment(
  attachments: Attachment[],
): Attachment | undefined {
  return attachments.find(
    (attachment) =>
      attachment.type === 'image'
      && Boolean(attachment.base64)
      && !normalizeSupportedImageMimeType(attachment.mimeType),
  );
}
