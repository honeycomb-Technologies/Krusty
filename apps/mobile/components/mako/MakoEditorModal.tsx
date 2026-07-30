import { useEffect, useState } from "react";
import {
  Modal,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";

interface MakoEditorModalProps {
  visible: boolean;
  title: string;
  subtitle?: string;
  initialValue: string;
  isSaving: boolean;
  onClose: () => void;
  onSave: (content: string) => Promise<void>;
}

export function MakoEditorModal({
  visible,
  title,
  subtitle,
  initialValue,
  isSaving,
  onClose,
  onSave,
}: MakoEditorModalProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [value, setValue] = useState(initialValue);

  useEffect(() => {
    if (visible) {
      setValue(initialValue);
    }
  }, [initialValue, visible]);

  const trimmed = value.trim();

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View
          style={[
            styles.panel,
            {
              backgroundColor: t.surfaceOverlayStrong,
              borderColor: t.border,
            },
          ]}
        >
          <View style={[styles.header, { borderBottomColor: t.border }]}>
            <View style={styles.copy}>
              <Text style={[styles.title, { color: t.foreground }]}>{title}</Text>
              {subtitle ? (
                <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
                  {subtitle}
                </Text>
              ) : null}
            </View>
          </View>

          <TextInput
            multiline
            autoFocus
            value={value}
            onChangeText={setValue}
            placeholder="Write the current operating note..."
            placeholderTextColor={`${t.mutedForeground}aa`}
            style={[
              styles.input,
              {
                color: t.foreground,
                borderColor: t.border,
                backgroundColor: t.card,
              },
            ]}
          />

          <View style={[styles.actions, { borderTopColor: t.border }]}>
            <Pressable onPress={onClose} style={styles.action}>
              <Text style={[styles.actionText, { color: t.mutedForeground }]}>
                Cancel
              </Text>
            </Pressable>
            <Pressable
              onPress={async () => {
                if (!trimmed || isSaving) {
                  return;
                }
                await onSave(trimmed);
              }}
              disabled={!trimmed || isSaving}
              style={styles.action}
            >
              <Text
                style={[
                  styles.actionText,
                  { color: !trimmed || isSaving ? `${t.userMessage}88` : t.userMessage },
                ]}
              >
                {isSaving ? "Saving..." : "Save"}
              </Text>
            </Pressable>
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.44)",
    justifyContent: "flex-end",
    padding: 12,
  },
  panel: {
    width: "100%",
    maxHeight: "88%",
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  header: {
    paddingHorizontal: 16,
    paddingTop: 16,
    paddingBottom: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  copy: {
    gap: 4,
  },
  title: {
    fontSize: 18,
    fontWeight: "600",
    letterSpacing: -0.3,
  },
  subtitle: {
    fontSize: 12,
    lineHeight: 18,
  },
  input: {
    minHeight: 260,
    margin: 16,
    paddingHorizontal: 14,
    paddingVertical: 14,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    fontSize: 14,
    lineHeight: 20,
    textAlignVertical: "top",
  },
  actions: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  action: {
    paddingVertical: 4,
  },
  actionText: {
    fontSize: 13,
    fontWeight: "600",
  },
});
