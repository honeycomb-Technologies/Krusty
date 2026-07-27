import { useEffect, type ReactNode } from "react";
import {
  StyleSheet,
  View,
  type DimensionValue,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import Animated, {
  Easing,
  cancelAnimation,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import { useThemeContext } from "../../hooks/useTheme";

type SkeletonTone = "default" | "soft" | "strong";

export interface SkeletonProps {
  width?: DimensionValue;
  height?: number;
  radius?: number;
  tone?: SkeletonTone;
  style?: StyleProp<ViewStyle>;
}

function toneAlpha(tone: SkeletonTone, scheme: "dark" | "light"): number {
  if (scheme === "dark") {
    if (tone === "soft") return 0.08;
    if (tone === "strong") return 0.2;
    return 0.14;
  }
  if (tone === "soft") return 0.06;
  if (tone === "strong") return 0.16;
  return 0.1;
}

export function Skeleton({
  width = "100%",
  height = 14,
  radius = 10,
  tone = "default",
  style,
}: SkeletonProps) {
  const { theme } = useThemeContext();
  const reduceMotion = useReducedMotion();
  const pulse = useSharedValue(reduceMotion ? 0.72 : 0.45);

  useEffect(() => {
    if (reduceMotion) {
      pulse.value = 0.72;
      return;
    }

    pulse.value = withRepeat(
      withTiming(1, {
        duration: 900,
        easing: Easing.inOut(Easing.quad),
      }),
      -1,
      true,
    );

    return () => cancelAnimation(pulse);
  }, [pulse, reduceMotion]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: pulse.value,
  }));

  const base = theme.scheme === "dark" ? "#FFFFFF" : "#111111";
  const backgroundColor = `${base}${Math.round(
    toneAlpha(tone, theme.scheme) * 255,
  )
    .toString(16)
    .padStart(2, "0")}`;

  return (
    <Animated.View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        {
          width,
          height,
          borderRadius: radius,
          backgroundColor,
        },
        animatedStyle,
        style,
      ]}
    />
  );
}

export function SkeletonLine({
  width = "100%",
  height = 12,
  tone = "default",
  style,
}: Omit<SkeletonProps, "radius">) {
  return (
    <Skeleton
      width={width}
      height={height}
      radius={999}
      tone={tone}
      style={style}
    />
  );
}

export function SkeletonBlock({
  children,
  style,
}: {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
}) {
  return <View style={[styles.block, style]}>{children}</View>;
}

export function SessionListSkeleton({ count = 5 }: { count?: number }) {
  return (
    <SkeletonBlock style={styles.sessionList}>
      {Array.from({ length: count }, (_, index) => (
        <View key={`session-skel-${index}`} style={styles.sessionCard}>
          <SkeletonLine width="68%" height={15} />
          <View style={styles.sessionMeta}>
            <SkeletonLine width={54} height={11} tone="soft" />
            <SkeletonLine width={72} height={11} tone="soft" />
          </View>
        </View>
      ))}
    </SkeletonBlock>
  );
}

export function ConversationSkeleton() {
  return (
    <SkeletonBlock style={styles.conversation}>
      <View style={styles.userBubbleWrap}>
        <Skeleton width="58%" height={42} radius={16} />
      </View>
      <View style={styles.aiGroup}>
        <SkeletonLine width="42%" height={12} tone="soft" />
        <SkeletonLine width="92%" />
        <SkeletonLine width="86%" />
        <SkeletonLine width="74%" />
      </View>
      <View style={styles.userBubbleWrap}>
        <Skeleton width="46%" height={36} radius={16} />
      </View>
      <View style={styles.aiGroup}>
        <SkeletonLine width="38%" height={12} tone="soft" />
        <SkeletonLine width="90%" />
        <SkeletonLine width="80%" />
        <Skeleton width="100%" height={72} radius={14} tone="soft" />
      </View>
    </SkeletonBlock>
  );
}

export function ListRowsSkeleton({
  rows = 6,
  showAvatar = false,
}: {
  rows?: number;
  showAvatar?: boolean;
}) {
  return (
    <SkeletonBlock style={styles.listRows}>
      {Array.from({ length: rows }, (_, index) => (
        <View key={`row-skel-${index}`} style={styles.listRow}>
          {showAvatar ? <Skeleton width={34} height={34} radius={17} /> : null}
          <View style={styles.listRowCopy}>
            <SkeletonLine width={`${72 - (index % 3) * 8}%`} height={13} />
            <SkeletonLine width={`${58 - (index % 2) * 10}%`} height={11} tone="soft" />
          </View>
        </View>
      ))}
    </SkeletonBlock>
  );
}

export function DetailPaneSkeleton() {
  return (
    <SkeletonBlock style={styles.detailPane}>
      <SkeletonLine width="54%" height={18} />
      <SkeletonLine width="88%" height={12} tone="soft" />
      <SkeletonLine width="80%" height={12} tone="soft" />
      <Skeleton width="100%" height={120} radius={14} tone="soft" />
      <SkeletonLine width="72%" height={12} />
      <SkeletonLine width="64%" height={12} tone="soft" />
      <Skeleton width="100%" height={84} radius={14} />
    </SkeletonBlock>
  );
}

export function CardGridSkeleton({ cards = 3 }: { cards?: number }) {
  return (
    <SkeletonBlock style={styles.cardGrid}>
      {Array.from({ length: cards }, (_, index) => (
        <View key={`card-skel-${index}`} style={styles.metricCard}>
          <SkeletonLine width="42%" height={11} tone="soft" />
          <SkeletonLine width="56%" height={20} />
          <SkeletonLine width="70%" height={11} tone="soft" />
        </View>
      ))}
    </SkeletonBlock>
  );
}

export function MakoHomeSkeleton() {
  return (
    <SkeletonBlock style={styles.makoHome}>
      <View style={styles.makoStatus}>
        <SkeletonLine width="48%" height={12} tone="soft" />
        <Skeleton width={16} height={16} radius={8} />
      </View>
      <ConversationSkeleton />
    </SkeletonBlock>
  );
}

export function RunDetailSkeleton() {
  return (
    <SkeletonBlock style={styles.runDetail}>
      <View style={styles.runSummaryRow}>
        <Skeleton width="30%" height={58} radius={12} />
        <Skeleton width="30%" height={58} radius={12} />
        <Skeleton width="30%" height={58} radius={12} />
      </View>
      <SkeletonLine width="36%" height={14} />
      <ListRowsSkeleton rows={4} />
      <Skeleton width="100%" height={96} radius={14} tone="soft" />
    </SkeletonBlock>
  );
}

export function ChannelsSkeleton() {
  return (
    <SkeletonBlock style={styles.channels}>
      <View style={styles.channelsSummary}>
        <Skeleton width="48%" height={54} radius={12} />
        <Skeleton width="48%" height={54} radius={12} />
      </View>
      <ListRowsSkeleton rows={4} />
    </SkeletonBlock>
  );
}

const styles = StyleSheet.create({
  block: {
    width: "100%",
  },
  sessionList: {
    paddingHorizontal: 16,
    paddingTop: 8,
    gap: 12,
  },
  sessionCard: {
    borderRadius: 18,
    paddingHorizontal: 16,
    paddingVertical: 16,
    gap: 12,
  },
  sessionMeta: {
    flexDirection: "row",
    gap: 10,
  },
  conversation: {
    flex: 1,
    width: "100%",
    paddingHorizontal: 18,
    paddingTop: 18,
    gap: 18,
  },
  userBubbleWrap: {
    alignItems: "flex-end",
  },
  aiGroup: {
    gap: 8,
  },
  listRows: {
    gap: 14,
  },
  listRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  listRowCopy: {
    flex: 1,
    minWidth: 0,
    gap: 8,
  },
  detailPane: {
    gap: 12,
    padding: 16,
  },
  cardGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
  },
  metricCard: {
    width: "31%",
    minWidth: 96,
    flexGrow: 1,
    gap: 10,
    paddingVertical: 4,
  },
  makoHome: {
    flex: 1,
    gap: 12,
    paddingTop: 4,
  },
  makoStatus: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 16,
    paddingBottom: 8,
  },
  runDetail: {
    gap: 16,
    paddingHorizontal: 16,
    paddingTop: 12,
  },
  runSummaryRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 10,
  },
  channels: {
    gap: 16,
  },
  channelsSummary: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 10,
  },
});
