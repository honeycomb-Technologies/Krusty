import { Pressable, ScrollView, StyleSheet, Text } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";

interface HiveTopNavProps<T extends string> {
  items: Array<{ id: T; label: string }>;
  active: T;
  onSelect: (item: T) => void;
}

export function HiveTopNav<T extends string>({
  items,
  active,
  onSelect,
}: HiveTopNavProps<T>) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      style={styles.scroll}
      contentContainerStyle={[
        styles.content,
        {
          borderColor: t.glass.border,
        },
      ]}
    >
      {items.map((item) => {
        const isActive = item.id === active;
        return (
          <Pressable
            key={item.id}
            onPress={() => {
              if (!isActive) {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onSelect(item.id);
              }
            }}
            style={[
              styles.segment,
              {
                backgroundColor: isActive
                  ? t.glass.backgroundElevated
                  : "transparent",
                borderColor: t.glass.border,
              },
            ]}
          >
            <Text
              style={[
                styles.label,
                { color: isActive ? t.foreground : t.mutedForeground },
              ]}
            >
              {item.label}
            </Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flexGrow: 0,
    maxHeight: 58,
  },
  content: {
    alignItems: "center",
    gap: 0,
    paddingHorizontal: 16,
    paddingBottom: 8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    overflow: "hidden",
  },
  segment: {
    alignSelf: "flex-start",
    borderRightWidth: StyleSheet.hairlineWidth,
    minHeight: 40,
    justifyContent: "center",
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  label: {
    fontSize: 13,
    fontWeight: "600",
  },
});
