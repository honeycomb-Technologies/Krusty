import { useRouter } from "expo-router";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { SettingsPanel } from "../../components/settings/SettingsPanel";
import { SettingsHeader } from "../../components/settings/sections";
import { useThemeContext } from "../../hooks/useTheme";

export default function SettingsScreen() {
  const router = useRouter();
  const { theme } = useThemeContext();
  const handleClose = () => {
    if (router.canGoBack()) {
      router.back();
      return;
    }

    router.replace("/(tabs)");
  };

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: theme.colors.background }]}>
      <SettingsHeader onClose={handleClose} />
      <SettingsPanel showHeader={false} />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
});
