import { memo, useEffect, useState } from 'react';
import {
  Keyboard,
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  TextInput,
  View,
} from 'react-native';
import { ArrowUp } from 'lucide-react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { AppBottomSheet } from '../sheets/AppBottomSheet';

const WEB_INPUT_STYLE = Platform.OS === 'web'
  ? ({
      outlineStyle: 'none',
      outlineWidth: 0,
      resize: 'none',
    } as any)
  : null;

/**
 * The editor lives in a Modal-hosted full-height sheet, so RN's
 * KeyboardAvoidingView cannot measure its frame reliably. Track the keyboard
 * directly and pad the sheet body, mirroring the terminal quick bar.
 */
function useEditorKeyboardInset(): number {
  const [inset, setInset] = useState(0);

  useEffect(() => {
    if (Platform.OS === 'web') return;

    const showEvent = Platform.OS === 'ios'
      ? 'keyboardWillShow'
      : 'keyboardDidShow';
    const hideEvent = Platform.OS === 'ios'
      ? 'keyboardWillHide'
      : 'keyboardDidHide';
    const showSubscription = Keyboard.addListener(showEvent, (event) => {
      setInset(Math.max(0, event.endCoordinates.height));
    });
    const hideSubscription = Keyboard.addListener(hideEvent, () => setInset(0));
    return () => {
      showSubscription.remove();
      hideSubscription.remove();
    };
  }, []);

  return inset;
}

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
  onAccent: string;
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
  onAccent,
  border,
  keyboardAppearance,
}: ChatBarExpandedEditorProps) {
  const insets = useSafeAreaInsets();
  const keyboardInset = useEditorKeyboardInset();
  // The sheet already pads max(insets.bottom, 8); only the uncovered remainder
  // of the keyboard needs additional clearance.
  const keyboardPadding = Math.max(
    0,
    keyboardInset - Math.max(insets.bottom, 8),
  );

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
          contentStyle={[
            styles.expandedEditorContent,
            { paddingBottom: 12 + keyboardPadding },
          ]}
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
              <ArrowUp size={18} color={onAccent} strokeWidth={2.5} />
            </Pressable>
          </View>
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
  },
  expandedEditorInput: {
    flex: 1,
    minHeight: 120,
    paddingHorizontal: 2,
    paddingVertical: 10,
    fontSize: 16,
    lineHeight: 24,
  },
  expandedEditorFooter: {
    minHeight: 56,
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

export const ChatBarExpandedEditor = memo(
  ChatBarExpandedEditorComponent,
  (previous, next) => {
    if (!previous.visible && !next.visible) return true;
    return (
      previous.visible === next.visible &&
      previous.text === next.text &&
      previous.onChangeText === next.onChangeText &&
      previous.onClose === next.onClose &&
      previous.onSend === next.onSend &&
      previous.canSend === next.canSend &&
      previous.disabled === next.disabled &&
      previous.placeholder === next.placeholder &&
      previous.mutedForeground === next.mutedForeground &&
      previous.foreground === next.foreground &&
      previous.userMessage === next.userMessage &&
      previous.onAccent === next.onAccent &&
      previous.border === next.border &&
      previous.keyboardAppearance === next.keyboardAppearance
    );
  },
);
