import { useState } from 'react';
import { View, Text, Pressable, StyleSheet } from 'react-native';
import { Brain, ChevronDown, ChevronRight } from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { MarkdownContent } from './MarkdownContent';
import { ToolCallCard } from './ToolCallCard';
import type { ChatMessage } from '@krusty/api';

interface MessageBubbleProps {
  message: ChatMessage;
  isLast: boolean;
  isStreaming: boolean;
  isThinking?: boolean;
}

export function MessageBubble({ message, isLast, isStreaming, isThinking }: MessageBubbleProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isUser = message.role === 'user';

  const toolCalls = message.toolCalls ?? [];
  const delegatedTools = toolCalls.filter(tc => tc.name === 'explore' || tc.name === 'build');
  const standardTools = toolCalls.filter(tc => tc.name !== 'explore' && tc.name !== 'build');

  return (
    <View style={[styles.container, isUser && styles.containerUser]}>
      {/* User message */}
      {isUser && message.content.length > 0 && (
        <View style={[styles.userBubble, { backgroundColor: t.userMessage }]}>
          <MarkdownContent content={message.content} isUser />
        </View>
      )}

      {/* Assistant message */}
      {!isUser && (
        <>
          {/* Thinking block */}
          {(message.thinking || (isLast && isThinking)) && (
            <ThinkingBlock
              content={message.thinking ?? ''}
              isStreaming={isLast && (isThinking || (isStreaming && !message.content))}
            />
          )}

          {/* Delegated tool calls */}
          {delegatedTools.length > 0 && (
            <View style={styles.toolSection}>
              {delegatedTools.map(tc => (
                <ToolCallCard key={tc.id} toolCall={tc} isStreaming={isLast && isStreaming} />
              ))}
            </View>
          )}

          {/* Standard tool calls */}
          {standardTools.length > 0 && (
            <View style={styles.toolSection}>
              {standardTools.length > 0 && (
                <Text style={[styles.toolLabel, { color: t.mutedForeground }]}>Actions</Text>
              )}
              {standardTools.map(tc => (
                <ToolCallCard key={tc.id} toolCall={tc} isStreaming={isLast && isStreaming} />
              ))}
            </View>
          )}

          {/* Response text — full width, no bubble */}
          {message.content.length > 0 && (
            <View style={styles.assistantText}>
              <MarkdownContent content={message.content} />
              {isLast && isStreaming && (
                <View style={[styles.cursor, { backgroundColor: t.userMessage }]} />
              )}
            </View>
          )}
        </>
      )}
    </View>
  );
}

function ThinkingBlock({ content, isStreaming }: { content: string; isStreaming: boolean }) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState(false);

  return (
    <Pressable onPress={() => setExpanded(!expanded)} style={[styles.thinkingBlock, { borderColor: t.thinking + '30' }]}>
      <View style={styles.thinkingHeader}>
        <Brain size={14} color={t.thinking} strokeWidth={2} />
        <Text style={[styles.thinkingLabel, { color: t.thinking }]}>
          {isStreaming ? 'Thinking...' : 'Thinking'}
        </Text>
        {expanded ? <ChevronDown size={14} color={t.thinking} /> : <ChevronRight size={14} color={t.thinking} />}
      </View>
      {expanded && (
        <Text style={[styles.thinkingContent, { color: t.mutedForeground }]} selectable>
          {content}
        </Text>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  container: {
    marginVertical: 6,
    gap: 6,
  },
  containerUser: {
    alignItems: 'flex-end',
  },

  // User bubble
  userBubble: {
    maxWidth: '85%',
    borderRadius: 20,
    borderBottomRightRadius: 6,
    paddingHorizontal: 16,
    paddingVertical: 10,
  },

  // Assistant text — full width, no bubble
  assistantText: {
    paddingVertical: 4,
  },
  cursor: {
    width: 2,
    height: 18,
    borderRadius: 1,
    marginTop: 4,
    opacity: 0.8,
  },

  // Tool section
  toolSection: {
    gap: 2,
  },
  toolLabel: {
    fontSize: 12,
    fontWeight: '500',
    marginBottom: 2,
    marginLeft: 4,
  },

  // Thinking
  thinkingBlock: {
    borderLeftWidth: 2,
    paddingLeft: 12,
    paddingVertical: 4,
  },
  thinkingHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  thinkingLabel: {
    fontSize: 13,
    fontWeight: '600',
    flex: 1,
  },
  thinkingContent: {
    fontSize: 13,
    lineHeight: 19,
    marginTop: 6,
    fontFamily: 'Courier',
  },
});
