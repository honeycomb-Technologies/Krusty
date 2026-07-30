import { ActivityIndicator, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { useThemeContext } from "../../../hooks/useTheme";
import { MitsuroLogo } from "../../../components/brand";
import { styles } from "./styles";

interface ChatBootScreenProps {
  status: string;
  isConfigured: boolean;
  connectionError: string | null;
  showLogo?: boolean;
  onRetryConnection: () => void;
  onOpenSetup: () => void;
}

export function ChatBootScreen({
  status,
  isConfigured,
  connectionError,
  showLogo = true,
  onRetryConnection,
  onOpenSetup,
}: ChatBootScreenProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isRetryable = status === "error" || status === "disconnected";

  return (
    <SafeAreaView style={[styles.bootScreen, { backgroundColor: t.background }]}> 
      <View style={styles.bootInner}>
        {showLogo ? <MitsuroLogo /> : null}
        {status === "connecting" ? (
          <>
            <ActivityIndicator
              size="small"
              color={t.userMessage}
              style={showLogo ? styles.bootSpinner : undefined}
            />
            <Text style={[styles.bootMessage, { color: t.mutedForeground }]}> 
              Reconnecting to your server...
            </Text>
          </>
        ) : null}
        {isRetryable ? (
          <View style={styles.bootActions}>
            <Text
              style={[
                styles.bootMessage,
                {
                  color: isConfigured ? t.error : t.mutedForeground,
                  marginTop: 0,
                },
              ]}
            >
              {connectionError ||
                (isConfigured
                  ? "Could not reconnect to your server."
                  : "Server connection is not configured.")}
            </Text>
            <Pressable
              accessibilityLabel={isConfigured ? "Retry connection" : "Open server setup"}
              accessibilityRole="button"
              onPress={isConfigured ? onRetryConnection : onOpenSetup}
              style={[styles.bootButton, { backgroundColor: t.userMessage }]}
            >
              <Text style={styles.bootButtonText}>
                {isConfigured ? "Retry Connection" : "Open Setup"}
              </Text>
            </Pressable>
            {isConfigured ? (
              <Pressable
                accessibilityLabel="Open server setup"
                accessibilityRole="button"
                onPress={onOpenSetup}
                style={[
                  styles.bootButtonSecondary,
                  { borderColor: t.border },
                ]}
              >
                <Text
                  style={[
                    styles.bootButtonSecondaryText,
                    { color: t.foreground },
                  ]}
                >
                  Server Setup
                </Text>
              </Pressable>
            ) : null}
          </View>
        ) : null}
      </View>
    </SafeAreaView>
  );
}
