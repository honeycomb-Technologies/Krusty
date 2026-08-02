import { Pressable, StyleSheet, Text, View } from "react-native";
import type { HiveCrewRuntimeMember } from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveStatusBadge } from "./HiveStatusBadge";

interface HiveCrewPickerProps {
  members: HiveCrewRuntimeMember[];
  selectedSlug?: string | null;
  isSaving?: boolean;
  onSelect: (slug: string | null) => void;
}

function crewSummary(member: HiveCrewRuntimeMember): string {
  const summary = member.identity?.preview?.trim();
  if (summary) {
    return summary;
  }
  const parts: string[] = [];
  if (member.active_run_count > 0) {
    parts.push(`${member.active_run_count} active`);
  }
  if (member.queued_task_count > 0) {
    parts.push(`${member.queued_task_count} queued`);
  }
  if (member.failed_run_count > 0) {
    parts.push(`${member.failed_run_count} failed`);
  }
  return parts.join(" • ") || "Distinct Hive Agent";
}

export function HiveCrewPicker({
  members,
  selectedSlug,
  isSaving = false,
  onSelect,
}: HiveCrewPickerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.list}>
      <Pressable
        disabled={isSaving}
        onPress={() => {
          onSelect(null);
        }}
        style={[styles.row, { borderColor: t.border }]}
      >
        <View style={styles.copy}>
          <Text style={[styles.title, { color: t.foreground }]}>Hive default</Text>
          <Text style={[styles.detail, { color: t.mutedForeground }]}>
            Use Hive&apos;s primary identity and home layers for this run.
          </Text>
        </View>
        <Text
          style={[
            styles.action,
            { color: selectedSlug ? t.userMessage : t.mutedForeground },
          ]}
        >
          {selectedSlug ? "Use" : isSaving ? "Saving..." : "Assigned"}
        </Text>
      </Pressable>

      {members.map((member) => {
        const isSelected = member.slug === selectedSlug;
        return (
          <Pressable
            key={member.slug}
            disabled={isSaving}
            onPress={() => {
              onSelect(member.slug);
            }}
            style={[styles.row, { borderColor: t.border }]}
          >
            <View style={styles.copy}>
              <View style={styles.header}>
                <Text style={[styles.title, { color: t.foreground }]}>{member.slug}</Text>
                <HiveStatusBadge status={member.status} />
              </View>
              <Text style={[styles.detail, { color: t.mutedForeground }]} numberOfLines={2}>
                {crewSummary(member)}
              </Text>
            </View>
            <Text
              style={[
                styles.action,
                { color: isSelected ? t.mutedForeground : t.userMessage },
              ]}
            >
              {isSelected ? (isSaving ? "Saving..." : "Assigned") : "Use"}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    gap: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 6,
  },
  copy: {
    flex: 1,
    minWidth: 0,
    gap: 4,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  title: {
    fontSize: 13,
    fontWeight: "600",
  },
  detail: {
    fontSize: 12,
    lineHeight: 17,
  },
  action: {
    fontSize: 12,
    fontWeight: "600",
    paddingTop: 2,
  },
});
