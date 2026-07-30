import { memo } from 'react';
import { Pressable, StyleSheet, View } from 'react-native';
import { ArrowUp, Mic } from 'lucide-react-native';

export interface ChatBarActionButtonProps {
  isStreaming: boolean;
  isRecording: boolean;
  canSend: boolean;
  mutedForeground: string;
  userMessage: string;
  error: string;
  onPress: () => void;
  onLongPress?: () => void;
}

function ChatBarActionButtonComponent({
  isStreaming,
  isRecording,
  canSend,
  mutedForeground,
  userMessage,
  error,
  onPress,
  onLongPress,
}: ChatBarActionButtonProps) {
  return (
    <Pressable
      onPress={onPress}
      onLongPress={canSend ? onLongPress : undefined}
      delayLongPress={300}
      style={({ pressed }) => [styles.actionBtn, {
        backgroundColor: isRecording
          ? error
          : canSend
            ? pressed ? userMessage + 'cc' : isStreaming ? error : userMessage
            : 'transparent',
      }]}
    >
      {isStreaming
        ? <View style={styles.stopGlyph} />
        : isRecording
          ? <View style={styles.stopGlyph} />
          : canSend
            ? <ArrowUp size={18} color="#fff" strokeWidth={2.5} />
            : <Mic size={18} color={mutedForeground} strokeWidth={1.8} />
      }
    </Pressable>
  );
}

const styles = StyleSheet.create({
  actionBtn: { width: 36, height: 36, borderRadius: 18, justifyContent: 'center', alignItems: 'center' },
  stopGlyph: {
    width: 12,
    height: 12,
    borderRadius: 2,
    backgroundColor: '#fff',
  },
});

export const ChatBarActionButton = memo(ChatBarActionButtonComponent);
