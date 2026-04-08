import { useState } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import { formatProjectLabel } from "./utils";

interface MakoSetCourseComposerProps {
  projectDir?: string | null;
  isSubmitting: boolean;
  onSubmit: (task: string, options?: { startAt?: string | null }) => Promise<void>;
}

type SchedulePreset = "now" | "30m" | "2h" | "tomorrow";

const SCHEDULE_PRESETS: Array<{ id: SchedulePreset; label: string }> = [
  { id: "now", label: "Now" },
  { id: "30m", label: "30m" },
  { id: "2h", label: "2h" },
  { id: "tomorrow", label: "Tomorrow" },
];

export function MakoSetCourseComposer({
  projectDir,
  isSubmitting,
  onSubmit,
}: MakoSetCourseComposerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [value, setValue] = useState("");
  const [schedulePreset, setSchedulePreset] = useState<SchedulePreset>("now");

  return (
    <GlassCard style={styles.card} elevated>
      <View style={styles.header}>
        <View style={styles.copy}>
          <Text style={[styles.title, { color: t.foreground }]}>Set course</Text>
          <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
            Give Mako a direction, a follow-up, or a correction.
          </Text>
        </View>
        <View
          style={[
            styles.projectPill,
            { borderColor: t.glass.border, backgroundColor: t.glass.background },
          ]}
        >
          <Text style={[styles.projectLabel, { color: t.mutedForeground }]}>
            {formatProjectLabel(projectDir)}
          </Text>
        </View>
      </View>

      <TextInput
        multiline
        value={value}
        onChangeText={setValue}
        placeholder="Keep the auth refactor moving and send me a checkpoint when tests are green."
        placeholderTextColor={`${t.mutedForeground}aa`}
        style={[
          styles.input,
          {
            color: t.foreground,
            backgroundColor: t.glass.background,
            borderColor: t.glass.border,
          },
        ]}
      />

      <View style={styles.scheduleRow}>
        <Text style={[styles.scheduleLabel, { color: t.mutedForeground }]}>When</Text>
        <View style={styles.scheduleOptions}>
          {SCHEDULE_PRESETS.map((preset) => {
            const selected = preset.id === schedulePreset;
            return (
              <Pressable
                key={preset.id}
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setSchedulePreset(preset.id);
                }}
                style={[
                  styles.scheduleChip,
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
                    styles.scheduleChipLabel,
                    { color: selected ? t.userMessage : t.mutedForeground },
                  ]}
                >
                  {preset.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      </View>

      <View style={styles.actions}>
        <Text style={[styles.hint, { color: t.mutedForeground }]}>
          {scheduleHint(schedulePreset)}
        </Text>
        <Pressable
          disabled={isSubmitting || value.trim().length === 0}
          onPress={async () => {
            const task = value.trim();
            if (!task) {
              return;
            }
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
            await onSubmit(task, { startAt: resolveStartAt(schedulePreset) });
            setValue("");
            setSchedulePreset("now");
          }}
          style={[
            styles.button,
            {
              backgroundColor:
                isSubmitting || value.trim().length === 0
                  ? `${t.userMessage}55`
                  : t.userMessage,
            },
          ]}
        >
          <Text style={styles.buttonLabel}>
            {isSubmitting
              ? "Setting..."
              : schedulePreset === "now"
                ? "Set course"
                : "Schedule course"}
          </Text>
        </Pressable>
      </View>
    </GlassCard>
  );
}

function resolveStartAt(preset: SchedulePreset): string | null {
  const now = new Date();
  switch (preset) {
    case "now":
      return null;
    case "30m":
      return new Date(now.getTime() + 30 * 60 * 1000).toISOString();
    case "2h":
      return new Date(now.getTime() + 2 * 60 * 60 * 1000).toISOString();
    case "tomorrow": {
      const tomorrow = new Date(now);
      tomorrow.setDate(tomorrow.getDate() + 1);
      tomorrow.setHours(9, 0, 0, 0);
      return tomorrow.toISOString();
    }
  }
}

function scheduleHint(preset: SchedulePreset): string {
  switch (preset) {
    case "now":
      return "New work opens as a run inside Mako.";
    case "30m":
      return "Mako will queue this run and wake it in 30 minutes.";
    case "2h":
      return "Mako will queue this run and wake it in two hours.";
    case "tomorrow":
      return "Mako will queue this run for tomorrow morning.";
  }
}

const styles = StyleSheet.create({
  card: {
    marginHorizontal: 16,
  },
  header: {
    flexDirection: "row",
    gap: 12,
    alignItems: "flex-start",
  },
  copy: {
    flex: 1,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
  },
  subtitle: {
    marginTop: 4,
    fontSize: 13,
    lineHeight: 18,
  },
  projectPill: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 6,
    maxWidth: 150,
  },
  projectLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  input: {
    minHeight: 110,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: 14,
    paddingHorizontal: 14,
    paddingVertical: 14,
    fontSize: 15,
    lineHeight: 22,
    textAlignVertical: "top",
  },
  scheduleRow: {
    marginTop: 14,
    gap: 10,
  },
  scheduleLabel: {
    fontSize: 12,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.3,
  },
  scheduleOptions: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  scheduleChip: {
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  scheduleChipLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  actions: {
    marginTop: 14,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  hint: {
    flex: 1,
    fontSize: 12,
    lineHeight: 17,
  },
  button: {
    borderRadius: 999,
    paddingHorizontal: 16,
    paddingVertical: 10,
  },
  buttonLabel: {
    color: "#ffffff",
    fontSize: 13,
    fontWeight: "700",
  },
});
