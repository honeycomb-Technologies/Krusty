import { Pressable, StyleSheet, Text, View } from "react-native";
import type { ComponentType } from "react";
import {
  Code2,
  MessageCircle,
  MessagesSquare,
  Toolbox,
} from "lucide-react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { SessionType } from "@mitsuro/api";

import { useThemeContext } from "../../hooks/useTheme";
import * as Haptics from "../../platform/haptics";
import { HiveIcon } from "../brand";
import { AdaptiveMaterial } from "../ui/AdaptiveMaterial";

const MODES: Array<{
  id: SessionType;
  label: string;
  icon: ComponentType<{
    size?: number;
    color?: string;
    strokeWidth?: number;
  }>;
}> = [
  { id: "chat", label: "Chat", icon: MessageCircle },
  { id: "code", label: "Code", icon: Code2 },
  { id: "hive", label: "Hive", icon: HiveIcon },
];

interface MobileAppHeaderProps {
  mode: SessionType;
  title?: string | null;
  onModeChange: (mode: SessionType) => void;
  onOpenThreads: () => void;
  onOpenToolbox: () => void;
  onTitlePress?: () => void;
  onHeightChange?: (height: number) => void;
}

export function MobileAppHeader({
  mode,
  title,
  onModeChange,
  onOpenThreads,
  onOpenToolbox,
  onTitlePress,
  onHeightChange,
}: MobileAppHeaderProps) {
  const { theme } = useThemeContext();
  const insets = useSafeAreaInsets();
  const t = theme.colors;
  const visibleTitle = title?.trim() ?? "";
  const topInset = insets.top;

  const impact = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
  };

  return (
    <View
      style={[styles.root, { paddingTop: topInset }]}
      onLayout={(event) =>
        onHeightChange?.(
          Math.max(0, Math.ceil(event.nativeEvent.layout.height) - topInset),
        )}
    >
      {topInset > 0
        ? (
          <View
            pointerEvents="none"
            style={[
              styles.statusBarFill,
              { height: topInset, backgroundColor: t.background },
            ]}
          />
        )
        : null}
      <View style={[styles.header, visibleTitle ? styles.headerWithTitle : null]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Open threads"
          onPress={() => {
            impact();
            onOpenThreads();
          }}
          style={[
            styles.headerButton,
            {
              borderColor: t.glass.border,
            },
          ]}
        >
          <AdaptiveMaterial borderRadius={12} tone="regular" />
          <MessagesSquare
            size={19}
            color={t.thinking}
            strokeWidth={1.9}
          />
        </Pressable>

        <View style={styles.centerStack}>
          <View
            accessibilityRole="tablist"
            style={[
              styles.modeIsland,
              {
                borderColor: t.glass.border,
              },
            ]}
          >
            <AdaptiveMaterial borderRadius={12} tone="regular" />
            {MODES.map((item) => {
              const active = item.id === mode;
              const Icon = item.icon;
              return (
                <Pressable
                  key={item.id}
                  accessibilityRole="button"
                  accessibilityLabel={`Switch to ${item.label}`}
                  accessibilityState={{ selected: active }}
                  onPress={() => {
                    impact();
                    onModeChange(item.id);
                  }}
                  style={[
                    styles.modeButton,
                    active && {
                      backgroundColor: t.glass.backgroundElevated,
                      borderColor: `${t.userMessage}42`,
                    },
                  ]}
                >
                  <Icon
                    size={17}
                    strokeWidth={active ? 2.2 : 1.8}
                    color={t.thinking}
                  />
                  {active ? (
                    <Text
                      numberOfLines={1}
                      style={[styles.modeLabel, { color: t.foreground }]}
                    >
                      {item.label}
                    </Text>
                  ) : null}
                </Pressable>
              );
            })}
          </View>

          {visibleTitle ? (
            <Pressable
              disabled={!onTitlePress}
              accessibilityRole={onTitlePress ? "button" : undefined}
              accessibilityLabel={onTitlePress ? "Rename thread" : undefined}
              onPress={onTitlePress}
              style={[
                styles.titleTag,
                {
                  borderColor: t.glass.border,
                },
              ]}
            >
              <AdaptiveMaterial borderRadius={10} tone="regular" />
              <Text
                numberOfLines={1}
                style={[styles.titleText, { color: t.foreground }]}
              >
                {visibleTitle}
              </Text>
            </Pressable>
          ) : null}
        </View>

        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Open toolbox"
          onPress={() => {
            impact();
            onOpenToolbox();
          }}
          style={[
            styles.headerButton,
            {
              borderColor: t.glass.border,
            },
          ]}
        >
          <AdaptiveMaterial borderRadius={12} tone="regular" />
          <Toolbox
            size={19}
            color={t.thinking}
            strokeWidth={1.9}
          />
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    paddingHorizontal: 14,
    zIndex: 40,
    overflow: "visible",
  },
  statusBarFill: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
  },
  header: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 10,
    paddingTop: 4,
    paddingBottom: 2,
    overflow: "visible",
  },
  headerWithTitle: {
    minHeight: 88,
    paddingBottom: 10,
  },
  headerButton: {
    position: "relative",
    width: 40,
    height: 40,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
    alignItems: "center",
    justifyContent: "center",
  },
  modeIsland: {
    position: "relative",
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
    padding: 3,
    gap: 2,
  },
  centerStack: {
    flex: 1,
    minWidth: 0,
    alignItems: "center",
    overflow: "visible",
  },
  modeButton: {
    flexShrink: 1,
    height: 34,
    minWidth: 38,
    paddingHorizontal: 10,
    borderRadius: 9,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "transparent",
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
  },
  modeLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  titleTag: {
    position: "relative",
    marginTop: 8,
    maxWidth: "100%",
    minHeight: 32,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
    paddingHorizontal: 14,
    paddingVertical: 7,
    justifyContent: "center",
    alignItems: "center",
    zIndex: 41,
  },
  titleText: {
    fontSize: 14,
    fontWeight: "700",
    lineHeight: 18,
    letterSpacing: -0.1,
    textAlign: "center",
  },
});
