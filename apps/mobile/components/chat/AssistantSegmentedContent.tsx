import { memo, useMemo } from "react";
import { View, StyleSheet } from "react-native";
import { MarkdownContent } from "./MarkdownContent";
import { assistantRenderSegments } from "./assistantSegments";

interface AssistantSegmentedContentProps {
  messageId: string;
  content: string;
}

export function AssistantSegmentedContent({
  messageId,
  content,
}: AssistantSegmentedContentProps) {
  const segments = useMemo(
    () => assistantRenderSegments(messageId, content),
    [content, messageId],
  );

  return (
    <View style={styles.container}>
      {segments.map((segment) =>
        segment.content ? (
          <MemoizedMarkdownSegment key={segment.id} content={segment.content} />
        ) : null,
      )}
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

const styles = StyleSheet.create({
  container: {
    gap: 2,
  },
});
