import { useMemo, useState } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { Pencil, Plus } from "lucide-react-native";
import type { HiveGroup } from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import * as Haptics from "../../platform/haptics";
import { ListRowsSkeleton } from "../ui/Skeleton";
import { HiveGroupEditorModal } from "./HiveGroupEditorModal";
import { HiveGroupRoomView } from "./HiveGroupRoomView";
import type { HiveGroupsState } from "./hooks/useHiveGroups";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";
import { workerFallbackColor, workerInitials } from "./workerAppearance";

interface HiveGroupsViewProps {
  state: HiveGroupsState;
  workers: HiveWorkersState;
}

const MODE_LABELS: Record<string, string> = {
  workbench: "Workbench",
  roundtable: "Roundtable",
  direct: "Direct",
};

function GroupRow({
  group,
  onOpen,
  onEdit,
}: {
  group: HiveGroup;
  onOpen: () => void;
  onEdit: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const preview = group.members.slice(0, 4);
  const overflow = group.members.length - preview.length;

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open Group ${group.title}`}
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onOpen();
      }}
      style={[styles.row, { borderColor: t.border, backgroundColor: t.surface }]}
    >
      <View style={styles.avatarStack}>
        {preview.map((member, index) => {
          const color = member.avatar_color ?? workerFallbackColor(member.slug);
          return (
            <View
              key={member.worker_id}
              style={[
                styles.stackAvatar,
                {
                  backgroundColor: `${color}22`,
                  borderColor: `${color}66`,
                  marginLeft: index === 0 ? 0 : -8,
                },
              ]}
            >
              <Text style={[styles.stackAvatarText, { color }]}>
                {workerInitials(member.display_name)}
              </Text>
            </View>
          );
        })}
        {overflow > 0 ? (
          <View
            style={[
              styles.stackAvatar,
              { backgroundColor: t.surfaceElevated, borderColor: t.border, marginLeft: -8 },
            ]}
          >
            <Text style={[styles.stackAvatarText, { color: t.mutedForeground }]}>
              +{overflow}
            </Text>
          </View>
        ) : null}
      </View>
      <View style={styles.rowCopy}>
        <View style={styles.rowTitleLine}>
          <Text style={[styles.rowTitle, { color: t.foreground }]} numberOfLines={1}>
            {group.title}
          </Text>
          {group.active_turn_id ? (
            <View style={[styles.activeDot, { backgroundColor: t.success }]} />
          ) : null}
        </View>
        <Text style={[styles.rowMeta, { color: t.mutedForeground }]} numberOfLines={1}>
          {group.members.length} Worker{group.members.length === 1 ? "" : "s"} ·{" "}
          {MODE_LABELS[group.execution_mode] ?? group.execution_mode}
          {group.active_turn_id ? " · turn running" : ""}
        </Text>
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Edit Group ${group.title}`}
        onPress={(event) => {
          event.stopPropagation();
          onEdit();
        }}
        style={styles.editButton}
      >
        <Pencil size={15} color={t.mutedForeground} strokeWidth={1.8} />
      </Pressable>
    </Pressable>
  );
}

/**
 * Groups roster: rooms where Workers collaborate. Opening a row swaps this
 * view for the room surface; the list (and its polling) unmounts while the
 * room is open so only one heavy surface exists at a time.
 */
export function HiveGroupsView({ state, workers }: HiveGroupsViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [openGroupId, setOpenGroupId] = useState<string | null>(null);
  const [editorTarget, setEditorTarget] = useState<
    { kind: "create" } | { kind: "edit"; group: HiveGroup } | null
  >(null);

  const editingGroup = useMemo(
    () =>
      editorTarget?.kind === "edit"
        ? (state.groups.find((group) => group.id === editorTarget.group.id) ??
          editorTarget.group)
        : null,
    [editorTarget, state.groups],
  );

  if (openGroupId) {
    return (
      <HiveGroupRoomView
        groupId={openGroupId}
        onBack={() => {
          setOpenGroupId(null);
          void state.refresh();
        }}
      />
    );
  }

  return (
    <View style={styles.container}>
      <ScrollView contentContainerStyle={styles.listContent}>
        <View style={styles.headerRow}>
          <Text style={[styles.headerText, { color: t.mutedForeground }]}>
            Rooms where your Workers collaborate on one timeline.
          </Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Create a Group"
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setEditorTarget({ kind: "create" });
            }}
            style={[styles.newButton, { borderColor: t.border, backgroundColor: t.surface }]}
          >
            <Plus size={14} color={t.foreground} strokeWidth={2} />
            <Text style={[styles.newButtonText, { color: t.foreground }]}>New Group</Text>
          </Pressable>
        </View>

        {state.error ? (
          <Text style={[styles.errorText, { color: t.error }]}>{state.error}</Text>
        ) : null}

        {state.isLoading && state.groups.length === 0 ? (
          <ListRowsSkeleton rows={3} />
        ) : state.groups.length === 0 ? (
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            No Groups yet. Create one and pick the Workers who should sit in it.
          </Text>
        ) : (
          state.groups.map((group) => (
            <GroupRow
              key={group.id}
              group={group}
              onOpen={() => setOpenGroupId(group.id)}
              onEdit={() => setEditorTarget({ kind: "edit", group })}
            />
          ))
        )}
      </ScrollView>

      <HiveGroupEditorModal
        visible={editorTarget !== null}
        group={editingGroup}
        workers={workers}
        isSaving={state.isSaving}
        onClose={() => setEditorTarget(null)}
        onCreate={async (request) => {
          const created = await state.createGroup(request);
          setEditorTarget(null);
          setOpenGroupId(created.id);
        }}
        onUpdate={async (id, request) => {
          await state.updateGroup(id, request);
          setEditorTarget(null);
        }}
        onArchive={async (id) => {
          await state.archiveGroup(id);
          setEditorTarget(null);
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  listContent: {
    padding: 14,
    gap: 10,
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  headerText: {
    flex: 1,
    fontSize: 12,
    lineHeight: 17,
  },
  newButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 999,
    paddingHorizontal: 12,
    paddingVertical: 6,
  },
  newButtonText: {
    fontSize: 12,
    fontWeight: "700",
  },
  errorText: {
    fontSize: 12,
  },
  emptyText: {
    fontSize: 13,
    lineHeight: 19,
    textAlign: "center",
    marginTop: 28,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  avatarStack: {
    flexDirection: "row",
    alignItems: "center",
  },
  stackAvatar: {
    width: 26,
    height: 26,
    borderRadius: 13,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  stackAvatarText: {
    fontSize: 10,
    fontWeight: "700",
  },
  rowCopy: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  rowTitleLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  rowTitle: {
    fontSize: 14,
    fontWeight: "700",
    flexShrink: 1,
  },
  activeDot: {
    width: 7,
    height: 7,
    borderRadius: 4,
  },
  rowMeta: {
    fontSize: 12,
  },
  editButton: {
    padding: 6,
  },
});
