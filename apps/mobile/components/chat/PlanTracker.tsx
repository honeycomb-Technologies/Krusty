import { useCallback, useEffect, useRef } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import { CheckCircle2, Circle, ListChecks } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { BlurView } from "../../platform/blur";
import { useThemeContext } from "../../hooks/useTheme";
import { usePlanStore } from "../../hooks/useStores";
import { useBreakpoint } from "../../hooks/useBreakpoint";

interface PlanTrackerProps {
  onHeightChange?: (height: number) => void;
}

export function PlanTracker({ onHeightChange }: PlanTrackerProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const isDark = theme.scheme === "dark";
  const lastReportedHeightRef = useRef(0);

  const items = usePlanStore((s) => s.items) ?? [];
  const isVisible = usePlanStore((s) => s.isVisible) ?? false;
  const trackerVisible = isVisible && items.length > 0;

  useEffect(() => {
    if (trackerVisible || lastReportedHeightRef.current === 0) {
      return;
    }

    lastReportedHeightRef.current = 0;
    onHeightChange?.(0);
  }, [onHeightChange, trackerVisible]);

  const handleLayout = useCallback(({ nativeEvent }: any) => {
    const nextHeight = Math.ceil(nativeEvent.layout.height);
    if (lastReportedHeightRef.current === nextHeight) {
      return;
    }

    lastReportedHeightRef.current = nextHeight;
    onHeightChange?.(nextHeight);
  }, [onHeightChange]);

  if (!trackerVisible) return null;

  const completed = items.filter((i) => i.completed).length;
  const total = items.length;

  return (
    <View
      style={[styles.container, isDesktop ? styles.containerDesktop : styles.containerMobile]}
      onLayout={handleLayout}
    >
      <BlurView
        intensity={30}
        tint={isDark ? "systemMaterialDark" : "systemMaterialLight"}
        style={StyleSheet.absoluteFill}
      />
      <View
        style={[
          StyleSheet.absoluteFill,
          { backgroundColor: isDark ? "rgba(11,17,25,0.88)" : "rgba(255,255,255,0.88)" },
        ]}
      />

      {/* Header */}
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <ListChecks size={16} color={t.thinking} strokeWidth={1.8} />
        <Text style={[styles.headerTitle, { color: t.foreground }]}>Plan</Text>
        <Text style={[styles.headerCount, { color: t.mutedForeground }]}>
          {completed}/{total}
        </Text>
      </View>

      {/* Items */}
      <View style={styles.items}>
        {items.map((item) => (
          <Pressable
            key={item.id}
            onPress={() => {
              Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            }}
            style={styles.item}
          >
            {item.completed ? (
              <CheckCircle2 size={16} color={t.success} strokeWidth={2} />
            ) : (
              <Circle size={16} color={t.mutedForeground} strokeWidth={1.5} />
            )}
            <Text
              style={[
                styles.itemText,
                { color: item.completed ? t.mutedForeground : t.foreground },
                item.completed && styles.itemCompleted,
              ]}
              numberOfLines={2}
            >
              {item.content}
            </Text>
          </Pressable>
        ))}
      </View>

      {/* Progress bar */}
      <View style={[styles.progressTrack, { backgroundColor: `${t.border}44` }]}>
        <View
          style={[
            styles.progressFill,
            {
              width: `${(completed / total) * 100}%`,
              backgroundColor: completed === total ? t.success : t.thinking,
            },
          ]}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    borderRadius: 16,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.08)",
  },
  containerMobile: {
    position: "absolute",
    top: 8,
    left: 12,
    right: 12,
    zIndex: 50,
  },
  containerDesktop: {
    marginHorizontal: 12,
    marginBottom: 8,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerTitle: {
    flex: 1,
    fontSize: 14,
    fontWeight: "600",
  },
  headerCount: {
    fontSize: 12,
    fontWeight: "500",
  },
  items: {
    padding: 10,
    gap: 6,
    maxHeight: 200,
  },
  item: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
    paddingVertical: 4,
    paddingHorizontal: 4,
  },
  itemText: {
    flex: 1,
    fontSize: 13,
    lineHeight: 18,
  },
  itemCompleted: {
    textDecorationLine: "line-through",
    opacity: 0.6,
  },
  progressTrack: {
    height: 3,
    marginHorizontal: 14,
    marginBottom: 10,
    borderRadius: 1.5,
    overflow: "hidden",
  },
  progressFill: {
    height: "100%",
    borderRadius: 1.5,
  },
});
