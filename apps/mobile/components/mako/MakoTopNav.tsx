import { Pressable, ScrollView, StyleSheet, Text } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";

interface MakoTopNavProps<T extends string> {
  items: Array<{ id: T; label: string }>;
  active: T;
  onSelect: (item: T) => void;
}

export function MakoTopNav<T extends string>({
  items,
  active,
  onSelect,
}: MakoTopNavProps<T>) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.content}
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
              styles.pill,
              {
                backgroundColor: isActive
                  ? t.glass.backgroundElevated
                  : t.glass.background,
                borderColor: isActive ? t.glass.borderLight : t.glass.border,
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
  content: {
    gap: 8,
    paddingHorizontal: 16,
    paddingBottom: 8,
  },
  pill: {
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 9,
  },
  label: {
    fontSize: 14,
    fontWeight: "600",
  },
});
