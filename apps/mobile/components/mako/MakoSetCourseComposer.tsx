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
import { MakoPriorityPicker } from "./MakoPriorityPicker";
import { MakoSchedulePicker } from "./MakoSchedulePicker";
import type { MakoRunPriority } from "@krusty/api";
import {
  resolveScheduleSelection,
  type MakoSchedulePreset,
} from "./schedule";
import { formatProjectLabel } from "./utils";

interface MakoSetCourseComposerProps {
  projectDir?: string | null;
  isSubmitting: boolean;
  onSubmit: (task: string, options?: { startAt?: string | null; priority?: MakoRunPriority | null }) => Promise<void>;
}

export function MakoSetCourseComposer({
  projectDir,
  isSubmitting,
  onSubmit,
}: MakoSetCourseComposerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [value, setValue] = useState("");
  const [schedulePreset, setSchedulePreset] = useState<MakoSchedulePreset>("now");
  const [customSchedule, setCustomSchedule] = useState("");
  const [priority, setPriority] = useState<MakoRunPriority>("normal");
  const schedule = resolveScheduleSelection(schedulePreset, customSchedule);

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

      <MakoSchedulePicker
        value={schedulePreset}
        onChange={setSchedulePreset}
        customValue={customSchedule}
        onCustomValueChange={setCustomSchedule}
        customError={schedulePreset === "custom" ? schedule.error : null}
      />
      <MakoPriorityPicker value={priority} onChange={setPriority} />

      <View style={styles.actions}>
        <Pressable
          disabled={
            isSubmitting ||
            value.trim().length === 0 ||
            (schedulePreset === "custom" && schedule.error !== null)
          }
          onPress={async () => {
            const task = value.trim();
            if (!task) {
              return;
            }
            if (schedulePreset === "custom" && schedule.error) {
              return;
            }
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
            await onSubmit(task, {
              startAt: schedule.startAt,
              priority,
            });
            setValue("");
            setSchedulePreset("now");
            setCustomSchedule("");
            setPriority("normal");
          }}
          style={[
            styles.button,
            {
              backgroundColor:
                isSubmitting ||
                value.trim().length === 0 ||
                (schedulePreset === "custom" && schedule.error !== null)
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
  actions: {
    marginTop: 14,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: 12,
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
