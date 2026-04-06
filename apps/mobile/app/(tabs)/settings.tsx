import { useRouter } from "expo-router";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { SettingsPanel } from "../../components/settings/SettingsPanel";
import { useThemeContext } from "../../hooks/useTheme";

export default function SettingsScreen() {
  const router = useRouter();
  const { theme } = useThemeContext();

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: theme.colors.background }]}>
      <SettingsPanel onClose={() => router.back()} />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
});
