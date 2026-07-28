import { memo } from 'react';
import {
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  TextInput,
  View,
} from 'react-native';
import { ArrowUp } from 'lucide-react-native';
import { AppBottomSheet } from '../sheets/AppBottomSheet';

const WEB_INPUT_STYLE = Platform.OS === 'web'
  ? ({
      outlineStyle: 'none',
      outlineWidth: 0,
      resize: 'none',
    } as any)
  : null;

export interface ChatBarExpandedEditorProps {
  visible: boolean;
  text: string;
  onChangeText: (value: string) => void;
  onClose: () => void;
  onSend: () => void;
  canSend: boolean;
  disabled: boolean;
  placeholder: string;
  mutedForeground: string;
  foreground: string;
  userMessage: string;
  border: string;
  keyboardAppearance: 'light' | 'dark' | undefined;
}

function ChatBarExpandedEditorComponent({
  visible,
  text,
  onChangeText,
  onClose,
  onSend,
  canSend,
  disabled,
  placeholder,
  mutedForeground,
  foreground,
  userMessage,
  border,
  keyboardAppearance,
}: ChatBarExpandedEditorProps) {
  return (
    <Modal
      visible={visible}
      transparent
      animationType="none"
      onRequestClose={onClose}
    >
      <View style={styles.expandedEditorModal}>
        <AppBottomSheet
          visible={visible}
          onClose={onClose}
          accessibilityLabel="expanded message editor"
          contentStyle={styles.expandedEditorContent}
          footer={
            <View style={styles.expandedEditorFooter}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Send expanded message"
                accessibilityState={{ disabled: !canSend }}
                disabled={!canSend}
                onPress={onSend}
                style={({ pressed }) => [
                  styles.expandedEditorSend,
                  {
                    backgroundColor: canSend
                      ? pressed
                        ? `${userMessage}cc`
                        : userMessage
                      : border,
                  },
                ]}
              >
                <ArrowUp size={18} color="#fff" strokeWidth={2.5} />
              </Pressable>
            </View>
          }
        >
          <TextInput
            autoFocus
            value={text}
            onChangeText={onChangeText}
            placeholder={placeholder}
            placeholderTextColor={`${mutedForeground}70`}
            multiline
            maxLength={500000}
            editable={!disabled}
            keyboardAppearance={keyboardAppearance}
            textAlignVertical="top"
            style={[
              styles.expandedEditorInput,
              WEB_INPUT_STYLE,
              {
                color: foreground,
              },
            ]}
          />
        </AppBottomSheet>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  expandedEditorModal: {
    flex: 1,
  },
  expandedEditorContent: {
    paddingHorizontal: 16,
    paddingBottom: 12,
  },
  expandedEditorInput: {
    flex: 1,
    minHeight: 220,
    paddingHorizontal: 2,
    paddingVertical: 10,
    fontSize: 16,
    lineHeight: 24,
  },
  expandedEditorFooter: {
    minHeight: 64,
    paddingHorizontal: 16,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
  },
  expandedEditorSend: {
    width: 42,
    height: 42,
    borderRadius: 14,
    alignItems: 'center',
    justifyContent: 'center',
  },
});

export const ChatBarExpandedEditor = memo(ChatBarExpandedEditorComponent);
