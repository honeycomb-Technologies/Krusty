import { useCallback, useState } from 'react';
import { Image, StyleSheet, Text, View, Pressable, Linking } from 'react-native';
import Markdown from '@ronradtke/react-native-markdown-display';
import * as Clipboard from '../../platform/clipboard';
import * as Haptics from '../../platform/haptics';
import { Copy } from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { ImagePreviewModal } from './ImagePreviewModal';

interface MarkdownContentProps {
  content: string;
  isUser?: boolean;
}

export function MarkdownContent({ content, isUser }: MarkdownContentProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [previewImage, setPreviewImage] = useState<{ uri: string; title?: string } | null>(null);

  const handleLink = useCallback((url: string) => {
    Linking.openURL(url);
    return false;
  }, []);

  const styles = getStyles(t, isUser);

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
              style={({ pressed, hovered }) => [
                markdownImageStyles.frame,
                {
                  borderColor: hovered ? t.userMessage : t.border,
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
        fence: (node, _children, _parent, markdownStyles) => {
          const lang = node.sourceInfo || '';
          const code = node.content || '';
          return (
            <View key={node.key} style={codeBlockStyles.container}>
              <View style={codeBlockStyles.header}>
                <Text style={[codeBlockStyles.lang, { color: t.mutedForeground }]}>{lang || 'code'}</Text>
                <Pressable
                  onPress={() => {
                    Clipboard.setStringAsync(code);
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  }}
                  hitSlop={8}
                >
                  <Copy size={14} color={t.mutedForeground} strokeWidth={1.5} />
                </Pressable>
              </View>
              <Text style={[codeBlockStyles.code, { color: t.foreground }]} selectable>
                {code}
              </Text>
            </View>
          );
        },
        code_inline: (node, _children, _parent, _styles) => (
          <Text
            key={node.key}
            style={{
              fontFamily: 'Courier',
              fontSize: 13,
              backgroundColor: isUser ? 'rgba(255,255,255,0.15)' : 'rgba(255,255,255,0.08)',
              color: t.foreground,
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
        {content}
      </Markdown>
      <ImagePreviewModal
        visible={Boolean(previewImage)}
        uri={previewImage?.uri}
        title={previewImage?.title}
        onClose={() => setPreviewImage(null)}
      />
    </>
  );
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

function getStyles(t: any, isUser?: boolean) {
  const textColor = isUser ? '#fff' : t.foreground;
  const mutedColor = isUser ? 'rgba(255,255,255,0.7)' : t.mutedForeground;

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
    link: { color: t.userMessage, textDecorationLine: 'underline' },
    blockquote: {
      borderLeftWidth: 3,
      borderLeftColor: t.userMessage + '60',
      paddingLeft: 12,
      marginVertical: 6,
      opacity: 0.85,
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
    backgroundColor: 'rgba(0,0,0,0.3)',
    borderRadius: 10,
    marginVertical: 6,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'rgba(255,255,255,0.08)',
  },
  header: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: 'rgba(255,255,255,0.06)',
  },
  lang: {
    fontSize: 11,
    fontFamily: 'Courier',
    fontWeight: '500',
    textTransform: 'uppercase',
  },
  code: {
    fontFamily: 'Courier',
    fontSize: 13,
    lineHeight: 19,
    padding: 12,
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
