import { Pressable, StyleSheet, Text, View } from "react-native";
import type { ComponentType } from "react";
import {
  Code2,
  MessageCircle,
  MessagesSquare,
  Toolbox,
} from "lucide-react-native";
import type { SessionType } from "@krusty/api";
import Animated, {
  Easing,
  FadeIn,
  FadeOut,
  LinearTransition,
} from "react-native-reanimated";

import { useThemeContext } from "../../hooks/useTheme";
import * as Haptics from "../../platform/haptics";
import { MakoSharkIcon } from "../ui/MakoSharkIcon";

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
  { id: "mako", label: "Mako", icon: MakoSharkIcon },
];

const AnimatedPressable = Animated.createAnimatedComponent(Pressable);
const modeLayoutTransition = LinearTransition.duration(180).easing(
  Easing.out(Easing.cubic),
);

interface MobileAppHeaderProps {
  mode: SessionType;
  title?: string | null;
  onModeChange: (mode: SessionType) => void;
  onOpenThreads: () => void;
  onOpenToolbox: () => void;
  onTitlePress?: () => void;
}

export function MobileAppHeader({
  mode,
  title,
  onModeChange,
  onOpenThreads,
  onOpenToolbox,
  onTitlePress,
}: MobileAppHeaderProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const visibleTitle = title?.trim() ?? "";

  const impact = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
  };

  return (
    <View style={styles.root}>
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
              backgroundColor: t.glass.background,
            },
          ]}
        >
          <MessagesSquare
            size={19}
            color={t.mutedForeground}
            strokeWidth={1.9}
          />
        </Pressable>

        <View style={styles.centerStack}>
          <View
            accessibilityRole="tablist"
            style={[
              styles.modeIsland,
              {
                backgroundColor: t.glass.background,
                borderColor: t.glass.border,
              },
            ]}
          >
            {MODES.map((item) => {
              const active = item.id === mode;
              const Icon = item.icon;
              return (
                <AnimatedPressable
                  key={item.id}
                  layout={modeLayoutTransition}
                  accessibilityRole="tab"
                  accessibilityLabel={item.label}
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
                    color={active ? t.foreground : t.mutedForeground}
                  />
                  {active ? (
                    <Animated.Text
                      entering={FadeIn.duration(140)}
                      exiting={FadeOut.duration(90)}
                      style={[styles.modeLabel, { color: t.foreground }]}
                    >
                      {item.label}
                    </Animated.Text>
                  ) : null}
                </AnimatedPressable>
              );
            })}
          </View>

          {visibleTitle ? (
            <Pressable
              disabled={!onTitlePress}
              accessibilityRole={onTitlePress ? "button" : undefined}
              accessibilityLabel={onTitlePress ? "Rename thread" : undefined}
              onPress={onTitlePress}
              style={styles.titleTuck}
            >
              <Text
                numberOfLines={1}
                style={[styles.titleText, { color: t.mutedForeground }]}
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
              backgroundColor: t.glass.background,
            },
          ]}
        >
          <Toolbox
            size={19}
            color={t.mutedForeground}
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
  },
  header: {
    height: 48,
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 10,
    paddingTop: 4,
  },
  headerWithTitle: {
    height: 64,
  },
  headerButton: {
    width: 40,
    height: 40,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  modeIsland: {
    flexDirection: "row",
    alignItems: "center",
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 3,
    gap: 2,
  },
  centerStack: {
    flex: 1,
    minWidth: 0,
    alignItems: "center",
  },
  modeButton: {
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
  titleTuck: {
    height: 18,
    maxWidth: "100%",
    justifyContent: "center",
    paddingHorizontal: 10,
  },
  titleText: {
    fontSize: 11,
    fontWeight: "600",
    letterSpacing: 0,
    textAlign: "center",
  },
});
