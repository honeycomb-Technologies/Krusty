import { Modal, Pressable, StyleSheet, View } from "react-native";
import { useThemeContext } from "../hooks/useTheme";
import { SettingsPanel } from "./settings/SettingsPanel";

interface SettingsModalProps {
  visible: boolean;
  onClose: () => void;
}

export function SettingsModal({ visible, onClose }: SettingsModalProps) {
  const { theme } = useThemeContext();

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View
          style={[
            styles.panel,
            {
              backgroundColor: theme.colors.surfaceOverlayStrong,
              borderColor: theme.colors.border,
            },
          ]}
        >
          <SettingsPanel active={visible} onClose={onClose} />
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.42)",
    justifyContent: "flex-end",
    padding: 12,
  },
  panel: {
    width: "100%",
    maxHeight: "92%",
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
});
