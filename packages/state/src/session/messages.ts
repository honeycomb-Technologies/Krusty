import type { ContentBlock } from '@krusty/api';

import {
  MAX_MESSAGE_CONTENT_LENGTH,
  SUPPORTED_IMAGE_MIME_TYPES,
} from './constants';
import { formatToolOutputForDisplay, parseDelegatedArtifactState } from './delegated';
import { buildStoredMessageId } from './transient';
import type {
  Attachment,
  ChatMessage,
  ChatMessageAttachment,
  ChatRenderPart,
  ToolCall,
} from './types';

function extensionForImageMimeType(mimeType: string | null | undefined): string {
  switch ((mimeType || '').trim().toLowerCase()) {
    case 'image/png':
      return 'png';
    case 'image/gif':
      return 'gif';
    case 'image/webp':
      return 'webp';
    default:
      return 'jpg';
  }
}

function parseImageAttachment(
  block: Record<string, any>,
  index: number,
): ChatMessageAttachment | null {
  const source = block.source;
  if (!source || typeof source !== 'object') return null;

  if (source.type === 'base64' && typeof source.data === 'string') {
    const mimeType =
      typeof source.media_type === 'string' ? source.media_type : 'image/png';
    return {
      type: 'image',
      name: `image-${index + 1}.${extensionForImageMimeType(mimeType)}`,
      mimeType,
      base64: source.data,
    };
  }

  if (source.type === 'url' && typeof source.url === 'string') {
    return {
      type: 'image',
      name: `image-${index + 1}`,
      uri: source.url,
    };
  }

  return null;
}

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

function appendRenderPart(
  message: ChatMessage,
  part: ChatRenderPart,
): void {
  message.renderParts = [...(message.renderParts || []), part];
}

/**
 * Join adjacent text into one render part.
 *
 * Must match the live stream path (`streaming.ts`): raw concatenation.
 * Stored history used to insert `\n` between consecutive text blocks, which
 * made reloaded messages diverge from the live timeline for the same content.
 */
function joinAdjacentText(existing: string, next: string): string {
  if (!existing) return next;
  if (!next) return existing;
  return existing + next;
}

function appendTextRenderPart(message: ChatMessage, content: string): void {
  if (!content) return;

  const parts = [...(message.renderParts || [])];
  const lastPart = parts[parts.length - 1];
  if (lastPart?.type === 'text') {
    parts[parts.length - 1] = {
      ...lastPart,
      content: joinAdjacentText(lastPart.content, content),
    };
  } else {
    parts.push({
      type: 'text',
      id: `text-${parts.length}`,
      content,
    });
  }
  message.renderParts = parts;
}

function appendThinkingRenderPart(message: ChatMessage, content: string): void {
  if (!content) return;

  const parts = [...(message.renderParts || [])];
  const lastPart = parts[parts.length - 1];
  if (lastPart?.type === 'thinking') {
    parts[parts.length - 1] = {
      ...lastPart,
      content: lastPart.content ? `${lastPart.content}\n\n${content}` : content,
    };
  } else {
    parts.push({
      type: 'thinking',
      id: `thinking-${parts.length}`,
      content,
    });
  }
  message.renderParts = parts;
}

function appendAttachmentsRenderPart(message: ChatMessage): void {
  const parts = message.renderParts || [];
  if (parts[parts.length - 1]?.type === 'attachments') return;
  if (parts.some((part) => part.type === 'attachments')) return;

  appendRenderPart(message, {
    type: 'attachments',
    id: `attachments-${parts.length}`,
  });
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
    renderParts: [],
  };
  const contentArray = Array.isArray(message.content) ? message.content : [];
  let imageIndex = 0;

  for (const block of contentArray) {
    if (!block || typeof block !== 'object') continue;

    if (block.type === 'text' || ('text' in block && !block.type)) {
      const text = block.text || '';
      if (parsed.content.length < MAX_MESSAGE_CONTENT_LENGTH) {
        parsed.content += (parsed.content ? '\n' : '') + text;
      }
      appendTextRenderPart(parsed, text);
    } else if (block.type === 'thinking' || 'thinking' in block) {
      const thinkingContent = block.thinking || '';
      parsed.thinking = parsed.thinking
        ? `${parsed.thinking}\n\n${thinkingContent}`
        : thinkingContent;
      appendThinkingRenderPart(parsed, thinkingContent);
    } else if (block.type === 'image') {
      const attachment = parseImageAttachment(block, imageIndex);
      if (attachment) {
        parsed.attachments = [...(parsed.attachments || []), attachment];
        appendAttachmentsRenderPart(parsed);
        imageIndex += 1;
      }
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
      appendRenderPart(parsed, {
        type: 'tool',
        id: `tool-${block.id}`,
        toolCallId: block.id,
      });
    }
  }

  if (
    !parsed.content
    && !parsed.thinking
    && (!parsed.toolCalls || parsed.toolCalls.length === 0)
  ) {
    parsed.content = extractTextContent(message.content);
    appendTextRenderPart(parsed, parsed.content);
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
    const hasAttachments = (parsed.attachments?.length ?? 0) > 0;
    if (hasContent || hasThinking || hasToolCalls || hasAttachments) {
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
