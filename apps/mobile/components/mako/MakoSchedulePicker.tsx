import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import {
  describeSchedulePreset,
  schedulePresetOptions,
  type MakoSchedulePreset,
} from "./schedule";

interface MakoSchedulePickerProps {
  value: MakoSchedulePreset;
  onChange: (value: MakoSchedulePreset) => void;
  includeImmediate?: boolean;
  label?: string;
  subject?: "course" | "run";
}

export function MakoSchedulePicker({
  value,
  onChange,
  includeImmediate = true,
  label = "When",
  subject = "course",
}: MakoSchedulePickerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const presets = schedulePresetOptions(includeImmediate);

  return (
    <View style={styles.wrap}>
      <Text style={[styles.label, { color: t.mutedForeground }]}>{label}</Text>
      <View style={styles.options}>
        {presets.map((preset) => {
          const selected = preset.id === value;
          return (
            <Pressable
              key={preset.id}
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onChange(preset.id);
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
                {preset.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <Text style={[styles.hint, { color: t.mutedForeground }]}>
        {describeSchedulePreset(value, subject)}
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
