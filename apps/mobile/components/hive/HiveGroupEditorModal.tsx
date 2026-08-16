import { useEffect, useMemo, useState } from "react";
import {
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import type {
  CreateHiveGroupRequest,
  HiveGroup,
  HiveGroupExecutionMode,
  UpdateHiveGroupRequest,
} from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";
import { workerAvatarColor, workerInitials } from "./workerAppearance";

const MODE_OPTIONS: Array<{
  value: HiveGroupExecutionMode;
  label: string;
  hint: string;
}> = [
  {
    value: "workbench",
    label: "Workbench",
    hint: "Every targeted Worker works in parallel, bounded by parallelism.",
  },
  {
    value: "roundtable",
    label: "Roundtable",
    hint: "Workers speak one at a time in rotating rounds.",
  },
  {
    value: "direct",
    label: "Direct",
    hint: "One assigned Worker handles each turn unless you @mention another.",
  },
];

interface HiveGroupEditorModalProps {
  visible: boolean;
  group: HiveGroup | null;
  workers: HiveWorkersState;
  isSaving: boolean;
  onClose: () => void;
  onCreate: (request: CreateHiveGroupRequest) => Promise<void>;
  onUpdate: (id: string, request: UpdateHiveGroupRequest) => Promise<void>;
  onArchive: (id: string) => Promise<void>;
}

function NumberField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  return (
    <View style={styles.numberField}>
      <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>{label}</Text>
      <TextInput
        value={value}
        onChangeText={(next) => onChange(next.replace(/[^0-9]/g, ""))}
        keyboardType="number-pad"
        style={[
          styles.numberInput,
          { color: t.foreground, borderColor: t.border, backgroundColor: t.surface },
        ]}
      />
    </View>
  );
}

/**
 * Create/edit surface for one Group: title, execution mode, turn caps, and
 * the ordered member roster picked from the existing Workers. Selection
 * order is roster order.
 */
export function HiveGroupEditorModal({
  visible,
  group,
  workers,
  isSaving,
  onClose,
  onCreate,
  onUpdate,
  onArchive,
}: HiveGroupEditorModalProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isCreate = group === null;

  const [title, setTitle] = useState("");
  const [mode, setMode] = useState<HiveGroupExecutionMode>("workbench");
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [assigneeId, setAssigneeId] = useState<string | null>(null);
  const [maxRounds, setMaxRounds] = useState("3");
  const [maxPosts, setMaxPosts] = useState("2");
  const [parallelism, setParallelism] = useState("3");
  const [contextWindow, setContextWindow] = useState("24");
  const [confirmingArchive, setConfirmingArchive] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setValidationError(null);
    setConfirmingArchive(false);
    if (group) {
      setTitle(group.title);
      setMode(group.execution_mode);
      setMemberIds(group.members.map((member) => member.worker_id));
      setAssigneeId(group.default_assignee_worker_id ?? null);
      setMaxRounds(String(group.max_rounds));
      setMaxPosts(String(group.max_member_messages_per_turn));
      setParallelism(String(group.parallelism));
      setContextWindow(String(group.context_window_messages));
    } else {
      setTitle("");
      setMode("workbench");
      setMemberIds([]);
      setAssigneeId(null);
      setMaxRounds("3");
      setMaxPosts("2");
      setParallelism("3");
      setContextWindow("24");
    }
  }, [group, visible]);

  const selectableWorkers = useMemo(
    () => workers.workers.filter((worker) => worker.status !== "archived"),
    [workers.workers],
  );

  const toggleMember = (workerId: string) => {
    setMemberIds((current) =>
      current.includes(workerId)
        ? current.filter((id) => id !== workerId)
        : [...current, workerId],
    );
    if (assigneeId === workerId) {
      setAssigneeId(null);
    }
  };

  const parsedCap = (value: string, fallback: number) => {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
  };

  const handleSave = async () => {
    const trimmedTitle = title.trim();
    if (!trimmedTitle) {
      setValidationError("Give the Group a title.");
      return;
    }
    if (memberIds.length === 0) {
      setValidationError("Pick at least one Worker.");
      return;
    }
    if (mode === "direct" && !assigneeId) {
      setValidationError("Direct Groups need a default assignee.");
      return;
    }
    setValidationError(null);
    const shared = {
      title: trimmedTitle,
      execution_mode: mode,
      max_rounds: parsedCap(maxRounds, 3),
      max_member_messages_per_turn: parsedCap(maxPosts, 2),
      parallelism: parsedCap(parallelism, 3),
      context_window_messages: parsedCap(contextWindow, 24),
      member_worker_ids: memberIds,
    };
    try {
      if (isCreate) {
        await onCreate({
          ...shared,
          default_assignee_worker_id: assigneeId ?? undefined,
        });
      } else if (group) {
        await onUpdate(group.id, {
          ...shared,
          // An empty value clears a previously set assignee.
          default_assignee_worker_id: assigneeId ?? "",
        });
      }
    } catch {
      // The owning state surfaces the error; keep the modal open for retry.
    }
  };

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose}>
        <Pressable
          style={[styles.sheet, { backgroundColor: t.background, borderColor: t.border }]}
          onPress={() => {}}
        >
          <Text style={[styles.title, { color: t.foreground }]}>
            {isCreate ? "New Group" : "Edit Group"}
          </Text>
          <ScrollView style={styles.scroll} contentContainerStyle={styles.scrollContent}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Title</Text>
            <TextInput
              value={title}
              onChangeText={setTitle}
              placeholder="Release war room"
              placeholderTextColor={t.mutedForeground}
              style={[
                styles.textInput,
                { color: t.foreground, borderColor: t.border, backgroundColor: t.surface },
              ]}
            />

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Execution mode</Text>
            <View style={styles.modeRow}>
              {MODE_OPTIONS.map((option) => {
                const selected = mode === option.value;
                return (
                  <Pressable
                    key={option.value}
                    onPress={() => setMode(option.value)}
                    style={[
                      styles.modeChip,
                      {
                        borderColor: selected ? t.userMessage : t.border,
                        backgroundColor: selected ? `${t.userMessage}18` : t.surface,
                      },
                    ]}
                  >
                    <Text
                      style={[
                        styles.modeChipText,
                        { color: selected ? t.userMessage : t.foreground },
                      ]}
                    >
                      {option.label}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
            <Text style={[styles.modeHint, { color: t.mutedForeground }]}>
              {MODE_OPTIONS.find((option) => option.value === mode)?.hint}
            </Text>

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>
              Workers · {memberIds.length} selected
            </Text>
            {selectableWorkers.length === 0 ? (
              <Text style={[styles.emptyWorkers, { color: t.mutedForeground }]}>
                Create Workers first — a Group is a room for existing Workers.
              </Text>
            ) : (
              selectableWorkers.map((worker) => {
                const selected = memberIds.includes(worker.id);
                const color = workerAvatarColor(worker);
                const isAssignee = assigneeId === worker.id;
                return (
                  <Pressable
                    key={worker.id}
                    onPress={() => toggleMember(worker.id)}
                    style={[
                      styles.workerRow,
                      {
                        borderColor: selected ? `${color}66` : t.border,
                        backgroundColor: selected ? `${color}12` : t.surface,
                      },
                    ]}
                  >
                    <View
                      style={[
                        styles.workerAvatar,
                        { backgroundColor: `${color}22`, borderColor: `${color}55` },
                      ]}
                    >
                      <Text style={[styles.workerAvatarText, { color }]}>
                        {workerInitials(worker.display_name)}
                      </Text>
                    </View>
                    <View style={styles.workerCopy}>
                      <Text style={[styles.workerName, { color: t.foreground }]} numberOfLines={1}>
                        {worker.display_name}
                      </Text>
                      <Text style={[styles.workerMeta, { color: t.mutedForeground }]} numberOfLines={1}>
                        @{worker.slug}
                        {worker.model ? ` · ${worker.model}` : ""}
                      </Text>
                    </View>
                    {mode === "direct" && selected ? (
                      <Pressable
                        onPress={(event) => {
                          event.stopPropagation();
                          setAssigneeId(isAssignee ? null : worker.id);
                        }}
                        style={[
                          styles.assigneeChip,
                          { borderColor: isAssignee ? t.userMessage : t.border },
                        ]}
                      >
                        <Text
                          style={[
                            styles.assigneeChipText,
                            { color: isAssignee ? t.userMessage : t.mutedForeground },
                          ]}
                        >
                          {isAssignee ? "Assignee" : "Assign"}
                        </Text>
                      </Pressable>
                    ) : null}
                  </Pressable>
                );
              })
            )}

            <View style={styles.capsRow}>
              {mode === "roundtable" ? (
                <NumberField label="Max rounds" value={maxRounds} onChange={setMaxRounds} />
              ) : null}
              {mode === "workbench" ? (
                <NumberField label="Parallelism" value={parallelism} onChange={setParallelism} />
              ) : null}
              <NumberField label="Posts per turn" value={maxPosts} onChange={setMaxPosts} />
              <NumberField label="Context msgs" value={contextWindow} onChange={setContextWindow} />
            </View>

            {validationError ? (
              <Text style={[styles.errorText, { color: t.error }]}>{validationError}</Text>
            ) : null}
            {workers.error ? (
              <Text style={[styles.errorText, { color: t.error }]}>{workers.error}</Text>
            ) : null}

            {!isCreate && group ? (
              <Pressable
                onPress={() => {
                  if (confirmingArchive) {
                    void onArchive(group.id);
                  } else {
                    setConfirmingArchive(true);
                  }
                }}
                style={[styles.archiveButton, { borderColor: t.error }]}
              >
                <Text style={[styles.archiveText, { color: t.error }]}>
                  {confirmingArchive
                    ? "Tap again to archive — the timeline and Workers survive"
                    : "Archive Group"}
                </Text>
              </Pressable>
            ) : null}
          </ScrollView>

          <View style={styles.footer}>
            <Pressable onPress={onClose} style={styles.footerButton}>
              <Text style={[styles.footerCancel, { color: t.mutedForeground }]}>Cancel</Text>
            </Pressable>
            <Pressable
              onPress={() => {
                void handleSave();
              }}
              disabled={isSaving}
              style={[
                styles.saveButton,
                { backgroundColor: isSaving ? `${t.userMessage}55` : t.userMessage },
              ]}
            >
              <Text style={[styles.saveText, { color: t.onAccent }]}>
                {isSaving ? "Saving…" : isCreate ? "Create Group" : "Save"}
              </Text>
            </Pressable>
          </View>
        </Pressable>
      </Pressable>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.45)",
    alignItems: "center",
    justifyContent: "center",
    padding: 18,
  },
  sheet: {
    width: "100%",
    maxWidth: 520,
    maxHeight: "88%",
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 16,
    gap: 10,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
  },
  scroll: {
    flexGrow: 0,
  },
  scrollContent: {
    gap: 8,
    paddingBottom: 8,
  },
  fieldLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
    marginTop: 6,
  },
  textInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 9,
    fontSize: 14,
  },
  modeRow: {
    flexDirection: "row",
    gap: 8,
  },
  modeChip: {
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 12,
    paddingVertical: 6,
  },
  modeChipText: {
    fontSize: 12,
    fontWeight: "700",
  },
  modeHint: {
    fontSize: 11,
    lineHeight: 16,
  },
  emptyWorkers: {
    fontSize: 12,
    lineHeight: 17,
  },
  workerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    borderWidth: 1,
    borderRadius: 12,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  workerAvatar: {
    width: 28,
    height: 28,
    borderRadius: 14,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  workerAvatarText: {
    fontSize: 11,
    fontWeight: "700",
  },
  workerCopy: {
    flex: 1,
    minWidth: 0,
  },
  workerName: {
    fontSize: 13,
    fontWeight: "600",
  },
  workerMeta: {
    fontSize: 11,
  },
  assigneeChip: {
    borderWidth: 1,
    borderRadius: 999,
    paddingHorizontal: 9,
    paddingVertical: 4,
  },
  assigneeChipText: {
    fontSize: 11,
    fontWeight: "700",
  },
  capsRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
    marginTop: 4,
  },
  numberField: {
    gap: 4,
    minWidth: 96,
  },
  numberInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 7,
    fontSize: 14,
    minWidth: 72,
  },
  errorText: {
    fontSize: 12,
    marginTop: 4,
  },
  archiveButton: {
    borderWidth: 1,
    borderRadius: 10,
    paddingVertical: 9,
    alignItems: "center",
    marginTop: 10,
  },
  archiveText: {
    fontSize: 12,
    fontWeight: "700",
  },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: 12,
  },
  footerButton: {
    paddingVertical: 8,
    paddingHorizontal: 6,
  },
  footerCancel: {
    fontSize: 13,
    fontWeight: "600",
  },
  saveButton: {
    borderRadius: 999,
    paddingHorizontal: 16,
    paddingVertical: 9,
  },
  saveText: {
    fontSize: 13,
    fontWeight: "700",
  },
});
