import { Pressable, StyleSheet, Text, View } from "react-native";
import { ArrowLeft, Menu } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoStatusBadge } from "./MakoStatusBadge";
import { getRuntimeLabel } from "./utils";

interface MakoTopBarProps {
  title: string;
  subtitle?: string;
  status: string;
  titleStatus?: string | null;
  showStatusBadge?: boolean;
  onBack?: () => void;
  onOpenMenu?: () => void;
}

export function MakoTopBar({
  title,
  subtitle,
  status,
  titleStatus,
  showStatusBadge = true,
  onBack,
  onOpenMenu,
}: MakoTopBarProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const hasAction = Boolean(onBack || onOpenMenu);
  const inlineStatus = titleStatus ? `(${getRuntimeLabel(titleStatus)})` : null;

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
          <View style={styles.titleRow}>
            <Text style={[styles.title, { color: t.foreground }]} numberOfLines={1}>
              {title}
            </Text>
            {inlineStatus ? (
              <Text
                style={[styles.titleStatus, { color: t.mutedForeground }]}
                numberOfLines={1}
              >
                {inlineStatus}
              </Text>
            ) : null}
          </View>
          {subtitle ? (
            <Text
              style={[styles.subtitle, { color: t.mutedForeground }]}
              numberOfLines={1}
            >
              {subtitle}
            </Text>
          ) : null}
        </View>

        {showStatusBadge ? <MakoStatusBadge status={status} /> : null}
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
  titleRow: {
    flexDirection: "row",
    alignItems: "baseline",
    gap: 8,
  },
  title: {
    fontSize: 22,
    fontWeight: "600",
    letterSpacing: -0.3,
    flexShrink: 1,
  },
  titleStatus: {
    fontSize: 13,
    fontWeight: "600",
    textTransform: "lowercase",
  },
  subtitle: {
    marginTop: 3,
    fontSize: 12,
    fontWeight: "500",
  },
});
