import { useState } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveCrewPicker } from "./HiveCrewPicker";
import { HivePriorityPicker } from "./HivePriorityPicker";
import { HiveSchedulePicker } from "./HiveSchedulePicker";
import type { HiveCrewRuntimeMember, HiveRunPriority } from "@mitsuro/api";
import {
  resolveScheduleSelection,
  type HiveSchedulePreset,
} from "./schedule";
import { formatProjectLabel } from "./utils";

interface HiveSetCourseComposerProps {
  projectDir?: string | null;
  crewMembers?: HiveCrewRuntimeMember[];
  isSubmitting: boolean;
  onSubmit: (
    task: string,
    options?: {
      startAt?: string | null;
      priority?: HiveRunPriority | null;
      crewSlug?: string | null;
    },
  ) => Promise<void>;
}

export function HiveSetCourseComposer({
  projectDir,
  crewMembers = [],
  isSubmitting,
  onSubmit,
}: HiveSetCourseComposerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [value, setValue] = useState("");
  const [schedulePreset, setSchedulePreset] = useState<HiveSchedulePreset>("now");
  const [customSchedule, setCustomSchedule] = useState("");
  const [priority, setPriority] = useState<HiveRunPriority>("normal");
  const [crewSlug, setCrewSlug] = useState<string | null>(null);
  const schedule = resolveScheduleSelection(schedulePreset, customSchedule);

  return (
    <View
      style={[
        styles.card,
        {
          borderColor: t.border,
        },
      ]}
    >
      <View style={styles.header}>
        <View style={styles.copy}>
          <Text style={[styles.title, { color: t.foreground }]}>Start a run</Text>
          <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
            Give Hive a direction, a follow-up, or a correction.
          </Text>
        </View>
        <View
          style={[
            styles.projectTag,
            { borderColor: t.border, backgroundColor: "transparent" },
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
            backgroundColor: t.card,
            borderColor: t.border,
          },
        ]}
      />

      <HiveSchedulePicker
        value={schedulePreset}
        onChange={setSchedulePreset}
        customValue={customSchedule}
        onCustomValueChange={setCustomSchedule}
        customError={schedulePreset === "custom" ? schedule.error : null}
      />
      <HivePriorityPicker value={priority} onChange={setPriority} />
      {crewMembers.length > 0 ? (
        <View style={styles.crewBlock}>
          <Text style={[styles.crewTitle, { color: t.foreground }]}>Run as</Text>
          <HiveCrewPicker
            members={crewMembers}
            selectedSlug={crewSlug}
            isSaving={isSubmitting}
            onSelect={setCrewSlug}
          />
        </View>
      ) : null}

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
              crewSlug,
            });
            setValue("");
            setSchedulePreset("now");
            setCustomSchedule("");
            setPriority("normal");
            setCrewSlug(null);
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
              ? "Starting..."
              : schedulePreset === "now"
                ? "Start run"
                : "Schedule run"}
          </Text>
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    marginHorizontal: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 12,
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
    fontSize: 15,
    fontWeight: "600",
  },
  subtitle: {
    marginTop: 4,
    fontSize: 12,
    lineHeight: 17,
  },
  projectTag: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 5,
    maxWidth: 150,
  },
  projectLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  input: {
    minHeight: 92,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: 12,
    paddingHorizontal: 12,
    paddingVertical: 12,
    fontSize: 14,
    lineHeight: 20,
    textAlignVertical: "top",
  },
  actions: {
    marginTop: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: 12,
  },
  button: {
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  buttonLabel: {
    color: "#ffffff",
    fontSize: 13,
    fontWeight: "600",
  },
  crewBlock: {
    marginTop: 12,
    gap: 8,
  },
  crewTitle: {
    fontSize: 12,
    fontWeight: "600",
  },
});
