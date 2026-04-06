import { Modal, Pressable, StyleSheet, View } from "react-native";

import { BlurView } from "../platform/blur";
import { useThemeContext } from "../hooks/useTheme";
import { SettingsPanel } from "./settings/SettingsPanel";

interface SettingsModalProps {
  visible: boolean;
  onClose: () => void;
}

export function SettingsModal({ visible, onClose }: SettingsModalProps) {
  const { theme } = useThemeContext();
  const isDark = theme.scheme === "dark";

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View style={styles.panel}>
          <BlurView
            intensity={40}
            tint={isDark ? "systemChromeMaterialDark" : "systemChromeMaterialLight"}
            style={StyleSheet.absoluteFill}
          />
          <View
            style={[
              StyleSheet.absoluteFill,
              {
                backgroundColor: isDark ? "rgba(11,17,25,0.94)" : "rgba(255,255,255,0.94)",
              },
            ]}
          />
          <SettingsPanel active={visible} onClose={onClose} />
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.5)",
    justifyContent: "center",
    alignItems: "center",
    padding: 32,
  },
  panel: {
    width: "100%",
    maxWidth: 480,
    maxHeight: "80%",
    borderRadius: 20,
    overflow: "hidden",
  },
});
