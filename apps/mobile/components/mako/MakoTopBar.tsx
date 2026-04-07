import { Pressable, StyleSheet, Text, View } from "react-native";
import { ArrowLeft, Menu } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoStatusBadge } from "./MakoStatusBadge";

interface MakoTopBarProps {
  title: string;
  subtitle?: string;
  status: string;
  onBack?: () => void;
  onOpenMenu?: () => void;
}

export function MakoTopBar({
  title,
  subtitle,
  status,
  onBack,
  onOpenMenu,
}: MakoTopBarProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const hasAction = Boolean(onBack || onOpenMenu);

  return (
    <View style={styles.wrap}>
      <View style={styles.row}>
        {hasAction ? (
          <Pressable
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onBack?.();
              onOpenMenu?.();
            }}
            style={styles.iconButton}
          >
            {onBack ? (
              <ArrowLeft size={18} color={t.foreground} strokeWidth={2} />
            ) : (
              <Menu size={20} color={t.foreground} strokeWidth={2} />
            )}
          </Pressable>
        ) : (
          <View style={styles.iconSpacer} />
        )}

        <View style={styles.copy}>
          <Text style={[styles.title, { color: t.foreground }]} numberOfLines={1}>
            {title}
          </Text>
          {subtitle ? (
            <Text
              style={[styles.subtitle, { color: t.mutedForeground }]}
              numberOfLines={1}
            >
              {subtitle}
            </Text>
          ) : null}
        </View>

        <MakoStatusBadge status={status} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    paddingHorizontal: 16,
    paddingVertical: 12,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  iconButton: {
    width: 32,
    height: 32,
    borderRadius: 16,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 2,
  },
  iconSpacer: {
    width: 32,
  },
  copy: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 24,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  subtitle: {
    marginTop: 3,
    fontSize: 13,
    fontWeight: "500",
  },
});
