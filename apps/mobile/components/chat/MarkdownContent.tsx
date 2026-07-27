import { memo, useCallback, useMemo, useState } from 'react';
import { Image, StyleSheet, Text, View, Pressable, Linking, ScrollView } from 'react-native';
import Markdown from '@ronradtke/react-native-markdown-display';
import * as Clipboard from '../../platform/clipboard';
import * as Haptics from '../../platform/haptics';
import { Copy } from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { ImagePreviewModal } from './ImagePreviewModal';
import { HtmlPreview } from './HtmlPreview';
import {
  hasClosedHtmlFence,
  isHtmlPreviewLanguage,
} from './htmlPreviewModel';

interface MarkdownContentProps {
  content: string;
  isUser?: boolean;
}

export const MarkdownContent = memo(function MarkdownContent({ content, isUser }: MarkdownContentProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [previewImage, setPreviewImage] = useState<{ uri: string; title?: string } | null>(null);
  const [hoveredImageKey, setHoveredImageKey] = useState<unknown>(null);
  const [copiedCodeKey, setCopiedCodeKey] = useState<unknown>(null);
  const renderContent = stabilizeStreamingMarkdown(content);

  const handleLink = useCallback((url: string) => {
    Linking.openURL(url);
    return false;
  }, []);

  const styles = useMemo(() => getStyles(t, isUser), [isUser, t]);
  const isDark = theme.scheme === 'dark';

  const renderCodeBlock = (node: any, language = '') => {
    const code = stripTrailingCodeNewline(node?.content);
    const copied = copiedCodeKey === node.key;

    if (
      !isUser &&
      isHtmlPreviewLanguage(language) &&
      hasClosedHtmlFence(content)
    ) {
      return <HtmlPreview key={node.key} html={code} />;
    }

    return (
      <View
        key={node.key}
        style={[
          codeBlockStyles.container,
          {
            backgroundColor: isDark ? '#090d12' : '#f3f5f7',
            borderColor: t.border,
          },
        ]}
      >
        <View
          style={[
            codeBlockStyles.header,
            {
              backgroundColor: isDark ? 'rgba(255,255,255,0.035)' : 'rgba(0,0,0,0.025)',
              borderBottomColor: t.border,
            },
          ]}
        >
          <Text style={[codeBlockStyles.lang, { color: t.mutedForeground }]}>
            {language || 'code'}
          </Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={copied ? 'Code copied' : 'Copy code'}
            onPress={() => {
              Clipboard.setStringAsync(code);
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setCopiedCodeKey(node.key);
              setTimeout(() => setCopiedCodeKey(null), 1200);
            }}
            hitSlop={8}
            style={({ pressed }) => [
              codeBlockStyles.copyButton,
              pressed && codeBlockStyles.copyButtonPressed,
            ]}
          >
            <Copy size={13} color={t.mutedForeground} strokeWidth={1.5} />
            <Text style={[codeBlockStyles.copyLabel, { color: t.mutedForeground }]}>
              {copied ? 'Copied' : 'Copy'}
            </Text>
          </Pressable>
        </View>
        <ScrollView
          horizontal
          directionalLockEnabled
          nestedScrollEnabled
          showsHorizontalScrollIndicator
          contentContainerStyle={codeBlockStyles.scrollContent}
          accessibilityLabel={`${language || 'Code'} block`}
        >
          <Text style={[codeBlockStyles.code, { color: t.foreground }]} selectable>
            {code}
          </Text>
        </ScrollView>
      </View>
    );
  };

  return (
    <>
      <Markdown
        style={styles}
        onLinkPress={handleLink}
        rules={{
        image: (node, _children, _parent, markdownStyles) => {
          const uri = getMarkdownImageUri(node);
          if (!uri) return null;
          const title = getMarkdownImageTitle(node);
          return (
            <Pressable
              key={node.key}
              onPress={(event) => {
                event.stopPropagation();
                Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                setPreviewImage({ uri, title });
              }}
              onHoverIn={() => setHoveredImageKey(node.key)}
              onHoverOut={() => setHoveredImageKey(null)}
              style={({ pressed }) => [
                markdownImageStyles.frame,
                {
                  borderColor:
                    hoveredImageKey === node.key ? t.userMessage : t.border,
                  opacity: pressed ? 0.88 : 1,
                },
              ]}
            >
              <Image
                source={{ uri }}
                style={[markdownStyles.image, markdownImageStyles.image]}
                resizeMode="contain"
                accessibilityLabel={title || 'Markdown image'}
              />
            </Pressable>
          );
        },
        fence: (node) => renderCodeBlock(node, node.sourceInfo || ''),
        code_block: (node) => renderCodeBlock(node),
        code_inline: (node, _children, _parent, _styles) => (
          <Text
            key={node.key}
            style={{
              fontFamily: 'Courier',
              fontSize: 13,
              backgroundColor: isUser ? `${t.userMessage}14` : 'rgba(255,255,255,0.08)',
              color: isUser ? t.userMessage : t.foreground,
              paddingHorizontal: 4,
              paddingVertical: 1,
              borderRadius: 4,
            }}
          >
            {node.content}
          </Text>
        ),
        }}
      >
        {renderContent}
      </Markdown>
      <ImagePreviewModal
        visible={Boolean(previewImage)}
        uri={previewImage?.uri}
        title={previewImage?.title}
        onClose={() => setPreviewImage(null)}
      />
    </>
  );
});

/**
 * Soft-stabilize incomplete markdown while streaming so punctuation/words do not
 * appear cut off or oddly spaced when a token arrives mid-construct.
 */
function stabilizeStreamingMarkdown(content: string): string {
  if (!content) return content;

  // Keep this deliberately conservative. Aggressive marker stripping can itself
  // make periods/words appear to vanish mid-stream.
  let next = content;

  // Incomplete ordered-list marker at end of stream ("1.") should not become a list item.
  next = next.replace(/(^|\n)(\d+)\.(\s*)$/u, "$1$2\\.$3");

  return next;
}

function getMarkdownImageUri(node: any): string | null {
  const attrs = node?.attributes ?? {};
  const uri = attrs.src ?? attrs.href ?? node?.src ?? node?.destination ?? node?.target;
  return typeof uri === 'string' && uri.trim() ? uri.trim() : null;
}

function getMarkdownImageTitle(node: any): string | undefined {
  const attrs = node?.attributes ?? {};
  const title = attrs.alt ?? attrs.title ?? node?.alt ?? node?.content;
  return typeof title === 'string' && title.trim() ? title.trim() : undefined;
}

function stripTrailingCodeNewline(content: unknown): string {
  if (typeof content !== 'string') return '';
  return content.endsWith('\n') ? content.slice(0, -1) : content;
}

function getStyles(t: any, isUser?: boolean) {
  const textColor = isUser ? t.userMessage : t.foreground;
  const mutedColor = isUser ? `${t.userMessage}b8` : t.mutedForeground;
  const accentColor = t.userMessage;

  return StyleSheet.create({
    body: { color: textColor, fontSize: 15, lineHeight: 22 },
    heading1: { color: textColor, fontSize: 22, fontWeight: '700', marginTop: 16, marginBottom: 8 },
    heading2: { color: textColor, fontSize: 19, fontWeight: '700', marginTop: 14, marginBottom: 6 },
    heading3: { color: textColor, fontSize: 17, fontWeight: '600', marginTop: 12, marginBottom: 4 },
    heading4: { color: textColor, fontSize: 15, fontWeight: '600', marginTop: 10, marginBottom: 4 },
    paragraph: { color: textColor, fontSize: 15, lineHeight: 22, marginVertical: 2 },
    strong: { fontWeight: '600' },
    em: { fontStyle: 'italic' },
    s: { textDecorationLine: 'line-through' },
    link: { color: accentColor, textDecorationLine: 'underline' },
    blockquote: {
      backgroundColor: 'transparent',
      borderColor: 'transparent',
      borderLeftWidth: 3,
      borderLeftColor: `${accentColor}60`,
      paddingLeft: 12,
      paddingHorizontal: 0,
      marginLeft: 0,
      marginVertical: 6,
      opacity: 0.85,
    },
    code_block: {
      color: textColor,
      backgroundColor: 'transparent',
      borderColor: 'transparent',
      borderWidth: 0,
      padding: 0,
    },
    fence: {
      color: textColor,
      backgroundColor: 'transparent',
      borderColor: 'transparent',
      borderWidth: 0,
      padding: 0,
    },
    bullet_list: { marginVertical: 2 },
    ordered_list: { marginVertical: 2 },
    list_item: { marginVertical: 1, flexDirection: 'row' },
    bullet_list_icon: { color: mutedColor, fontSize: 15, marginRight: 8 },
    ordered_list_icon: { color: mutedColor, fontSize: 15, marginRight: 8 },
    table: { borderWidth: StyleSheet.hairlineWidth, borderColor: t.border, borderRadius: 8, marginVertical: 6 },
    thead: { backgroundColor: 'rgba(255,255,255,0.05)' },
    th: { color: textColor, fontWeight: '600', padding: 8, fontSize: 13 },
    td: { color: textColor, padding: 8, fontSize: 13, borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: t.border },
    hr: { backgroundColor: t.border, height: StyleSheet.hairlineWidth, marginVertical: 8 },
    image: { borderRadius: 8 },
  });
}

const codeBlockStyles = StyleSheet.create({
  container: {
    borderRadius: 10,
    marginVertical: 6,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  lang: {
    fontSize: 11,
    fontFamily: 'Courier',
    fontWeight: '500',
    textTransform: 'uppercase',
  },
  copyButton: {
    minHeight: 28,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
    paddingHorizontal: 6,
    marginVertical: -4,
    borderRadius: 6,
  },
  copyButtonPressed: {
    opacity: 0.62,
  },
  copyLabel: {
    fontSize: 11,
    fontWeight: '500',
  },
  scrollContent: {
    minWidth: '100%',
    padding: 12,
  },
  code: {
    fontFamily: 'Courier',
    fontSize: 13,
    lineHeight: 19,
  },
});

const markdownImageStyles = StyleSheet.create({
  frame: {
    width: '100%',
    marginVertical: 8,
    borderRadius: 12,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    backgroundColor: 'rgba(255,255,255,0.06)',
  },
  image: {
    width: '100%',
    height: 220,
    borderRadius: 12,
  },
});
