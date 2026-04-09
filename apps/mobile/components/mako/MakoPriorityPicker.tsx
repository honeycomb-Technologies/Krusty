import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoRunPriority } from "@krusty/api";
import { describePriority, MAKO_PRIORITY_OPTIONS } from "./priority";

interface MakoPriorityPickerProps {
  value: MakoRunPriority;
  onChange: (value: MakoRunPriority) => void;
}

export function MakoPriorityPicker({
  value,
  onChange,
}: MakoPriorityPickerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.wrap}>
      <Text style={[styles.label, { color: t.mutedForeground }]}>Priority</Text>
      <View style={styles.options}>
        {MAKO_PRIORITY_OPTIONS.map((option) => {
          const selected = option.id === value;
          return (
            <Pressable
              key={option.id}
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onChange(option.id);
              }}
              style={[
                styles.chip,
                {
                  borderColor: selected ? t.userMessage : t.glass.border,
                  backgroundColor: selected
                    ? `${t.userMessage}22`
                    : t.glass.background,
                },
              ]}
            >
              <Text
                style={[
                  styles.chipLabel,
                  { color: selected ? t.userMessage : t.mutedForeground },
                ]}
              >
                {option.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <Text style={[styles.hint, { color: t.mutedForeground }]}>
        {describePriority(value)}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    gap: 10,
  },
  label: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  options: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  chip: {
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  chipLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  hint: {
    fontSize: 12,
    lineHeight: 17,
  },
});
