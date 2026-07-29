import { memo, useMemo } from "react";
import { Text, View, StyleSheet } from "react-native";
import { MarkdownContent } from "./MarkdownContent";
import {
  assistantRenderSegments,
  shouldSegmentAssistantContent,
} from "./assistantSegments";
import { useThemeContext } from "../../hooks/useTheme";

interface AssistantSegmentedContentProps {
  messageId: string;
  content: string;
  isStreaming?: boolean;
}

export function AssistantSegmentedContent({
  messageId,
  content,
  isStreaming = false,
}: AssistantSegmentedContentProps) {
  const shouldSegment = shouldSegmentAssistantContent(isStreaming);
  const segments = useMemo(
    () => shouldSegment ? assistantRenderSegments(messageId, content) : [],
    [content, messageId, shouldSegment],
  );

  if (!shouldSegment) {
    return <MarkdownContent content={content} />;
  }

  return (
    <View style={styles.container}>
      {segments.map((segment, index) => {
        if (!segment.content) return null;
        const isLiveTail = isStreaming && index === segments.length - 1;
        return isLiveTail ? (
          <LivePlainText key={segment.id} content={segment.content} />
        ) : (
          <MemoizedMarkdownSegment key={segment.id} content={segment.content} />
        );
      })}
    </View>
  );
}

const MemoizedMarkdownSegment = memo(function MemoizedMarkdownSegment({
  content,
}: {
  content: string;
}) {
  return <MarkdownContent content={content} />;
});

const LivePlainText = memo(function LivePlainText({ content }: { content: string }) {
  const { theme } = useThemeContext();
  return (
    <Text style={[styles.liveText, { color: theme.colors.foreground }]} selectable>
      {content}
    </Text>
  );
});

const styles = StyleSheet.create({
  container: {
    gap: 2,
  },
  liveText: {
    fontSize: 15,
    lineHeight: 22,
  },
});
