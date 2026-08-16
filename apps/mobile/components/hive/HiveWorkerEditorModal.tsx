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
  CreateHiveWorkerRequest,
  HiveWorker,
  HiveWorkerAutonomy,
  ModelInfo,
  UpdateHiveWorkerRequest,
} from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import { HIVE_WORKER_COLORS } from "./workerAppearance";

const AUTONOMY_OPTIONS: Array<{ value: HiveWorkerAutonomy; label: string }> = [
  { value: "manual", label: "Manual" },
  { value: "scheduled", label: "Scheduled" },
  { value: "always_on", label: "Always on" },
];

const PERMISSION_OPTIONS = [
  { value: "autonomous", label: "Autonomous" },
  { value: "supervised", label: "Supervised" },
];

export function slugFromWorkerName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/\s+/g, "-")
    .replace(/[^a-z0-9\-_]/g, "")
    .slice(0, 64);
}

export function isValidWorkerSlug(slug: string): boolean {
  return (
    slug.length > 0 && slug.length <= 64 && /^[a-z0-9\-_]+$/.test(slug)
  );
}

interface HiveWorkerEditorModalProps {
  visible: boolean;
  worker: HiveWorker | null;
  models: ModelInfo[];
  isSaving: boolean;
  onClose: () => void;
  onCreate: (request: CreateHiveWorkerRequest) => Promise<void>;
  onUpdate: (id: string, request: UpdateHiveWorkerRequest) => Promise<void>;
  onPause: (id: string) => Promise<void>;
  onResume: (id: string) => Promise<void>;
  onArchive: (id: string) => Promise<void>;
}

/**
 * Create/edit surface for one Hive Worker: name, slug (create only), pinned
 * model, permission mode, autonomy, and lifecycle actions. Persona documents
 * are edited from the roster row via the shared document editor.
 */
export function HiveWorkerEditorModal({
  visible,
  worker,
  models,
  isSaving,
  onClose,
  onCreate,
  onUpdate,
  onPause,
  onResume,
  onArchive,
}: HiveWorkerEditorModalProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isCreate = worker === null;

  const [displayName, setDisplayName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugEdited, setSlugEdited] = useState(false);
  const [avatarColor, setAvatarColor] = useState<string>(HIVE_WORKER_COLORS[0]);
  const [modelId, setModelId] = useState<string | null>(null);
  const [autonomy, setAutonomy] = useState<HiveWorkerAutonomy>("manual");
  const [permissionMode, setPermissionMode] = useState("autonomous");
  const [confirmingArchive, setConfirmingArchive] = useState(false);

  useEffect(() => {
    if (!visible) {
      return;
    }
    setDisplayName(worker?.display_name ?? "");
    setSlug(worker?.slug ?? "");
    setSlugEdited(false);
    setAvatarColor(worker?.avatar_color ?? HIVE_WORKER_COLORS[0]);
    setModelId(worker?.model ?? null);
    setAutonomy(worker?.autonomy ?? "manual");
    setPermissionMode(worker?.permission_mode ?? "autonomous");
    setConfirmingArchive(false);
  }, [visible, worker]);

  const effectiveSlug = isCreate
    ? slugEdited
      ? slug
      : slugFromWorkerName(displayName)
    : slug;
  const slugValid = !isCreate || isValidWorkerSlug(effectiveSlug);
  const nameValid = displayName.trim().length > 0;
  const canSave = nameValid && slugValid && !isSaving;

  const selectedModel = useMemo(
    () => models.find((candidate) => candidate.id === modelId) ?? null,
    [modelId, models],
  );

  const handleSave = async () => {
    if (!canSave) {
      return;
    }
    const modelFields = selectedModel
      ? { model: selectedModel.id, model_key: selectedModel.key ?? undefined }
      : {};
    if (isCreate) {
      await onCreate({
        slug: effectiveSlug,
        display_name: displayName.trim(),
        avatar_color: avatarColor,
        permission_mode: permissionMode,
        autonomy,
        ...modelFields,
      });
    } else if (worker) {
      await onUpdate(worker.id, {
        display_name: displayName.trim(),
        avatar_color: avatarColor,
        permission_mode: permissionMode,
        autonomy,
        ...modelFields,
      });
    }
  };

  const choiceRow = <Value extends string>(
    options: Array<{ value: Value; label: string }>,
    selected: Value,
    onSelect: (value: Value) => void,
  ) => (
    <View style={styles.choiceRow}>
      {options.map((option) => {
        const active = option.value === selected;
        return (
          <Pressable
            key={option.value}
            onPress={() => onSelect(option.value)}
            style={[
              styles.choice,
              {
                borderColor: active ? t.userMessage : t.border,
                backgroundColor: active ? `${t.userMessage}14` : t.card,
              },
            ]}
          >
            <Text
              style={[
                styles.choiceText,
                { color: active ? t.userMessage : t.mutedForeground },
              ]}
            >
              {option.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View
          style={[
            styles.panel,
            { backgroundColor: t.surfaceOverlayStrong, borderColor: t.border },
          ]}
        >
          <View style={[styles.header, { borderBottomColor: t.border }]}>
            <Text style={[styles.title, { color: t.foreground }]}>
              {isCreate ? "New Worker" : `Edit ${worker?.display_name ?? "Worker"}`}
            </Text>
            <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
              {isCreate
                ? "A Worker is a durable Hive identity with its own persona, model, and private DM."
                : "Changes to the model apply to this Worker's DM immediately."}
            </Text>
          </View>

          <ScrollView style={styles.body} contentContainerStyle={styles.bodyContent}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Name</Text>
            <TextInput
              value={displayName}
              onChangeText={setDisplayName}
              placeholder="Deep Researcher"
              placeholderTextColor={`${t.mutedForeground}aa`}
              style={[
                styles.input,
                { color: t.foreground, borderColor: t.border, backgroundColor: t.card },
              ]}
            />

            {isCreate ? (
              <>
                <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Slug</Text>
                <TextInput
                  value={effectiveSlug}
                  onChangeText={(value) => {
                    setSlugEdited(true);
                    setSlug(slugFromWorkerName(value));
                  }}
                  autoCapitalize="none"
                  autoCorrect={false}
                  placeholder="deep-researcher"
                  placeholderTextColor={`${t.mutedForeground}aa`}
                  style={[
                    styles.input,
                    {
                      color: t.foreground,
                      borderColor: slugValid || effectiveSlug.length === 0 ? t.border : t.error,
                      backgroundColor: t.card,
                    },
                  ]}
                />
              </>
            ) : null}

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Color</Text>
            <View style={styles.colorRow}>
              {HIVE_WORKER_COLORS.map((color) => (
                <Pressable
                  key={color}
                  onPress={() => setAvatarColor(color)}
                  style={[
                    styles.colorSwatch,
                    { backgroundColor: color },
                    avatarColor === color && {
                      borderWidth: 2,
                      borderColor: t.foreground,
                    },
                  ]}
                />
              ))}
            </View>

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Model</Text>
            <Text style={[styles.fieldHint, { color: t.mutedForeground }]}>
              {selectedModel
                ? `${selectedModel.display_name} · ${selectedModel.provider}`
                : "No pinned model — the DM keeps the server default."}
            </Text>
            <View style={[styles.modelList, { borderColor: t.border }]}>
              <ScrollView nestedScrollEnabled style={styles.modelScroll}>
                {models.map((model) => {
                  const active = model.id === modelId;
                  return (
                    <Pressable
                      key={`${model.provider}:${model.id}`}
                      onPress={() => setModelId(active ? null : model.id)}
                      style={[
                        styles.modelRow,
                        { borderBottomColor: t.border },
                        active && { backgroundColor: `${t.userMessage}14` },
                      ]}
                    >
                      <Text
                        numberOfLines={1}
                        style={[
                          styles.modelName,
                          { color: active ? t.userMessage : t.foreground },
                        ]}
                      >
                        {model.display_name}
                      </Text>
                      <Text style={[styles.modelProvider, { color: t.mutedForeground }]}>
                        {model.provider}
                      </Text>
                    </Pressable>
                  );
                })}
                {models.length === 0 ? (
                  <Text style={[styles.fieldHint, { color: t.mutedForeground }]}>
                    Model catalog unavailable.
                  </Text>
                ) : null}
              </ScrollView>
            </View>

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Permissions</Text>
            {choiceRow(PERMISSION_OPTIONS, permissionMode, setPermissionMode)}

            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Autonomy</Text>
            {choiceRow(AUTONOMY_OPTIONS, autonomy, setAutonomy)}

            {!isCreate && worker ? (
              <View style={[styles.lifecycle, { borderTopColor: t.border }]}>
                <Pressable
                  onPress={() => {
                    void (worker.status === "paused"
                      ? onResume(worker.id)
                      : onPause(worker.id));
                  }}
                  disabled={isSaving}
                  style={styles.lifecycleAction}
                >
                  <Text style={[styles.lifecycleText, { color: t.userMessage }]}>
                    {worker.status === "paused" ? "Resume Worker" : "Pause Worker"}
                  </Text>
                </Pressable>
                <Pressable
                  onPress={() => {
                    if (!confirmingArchive) {
                      setConfirmingArchive(true);
                      return;
                    }
                    void onArchive(worker.id);
                  }}
                  disabled={isSaving}
                  style={styles.lifecycleAction}
                >
                  <Text style={[styles.lifecycleText, { color: t.error }]}>
                    {confirmingArchive ? "Tap again to archive" : "Archive Worker"}
                  </Text>
                </Pressable>
              </View>
            ) : null}
          </ScrollView>

          <View style={[styles.actions, { borderTopColor: t.border }]}>
            <Pressable onPress={onClose} style={styles.action}>
              <Text style={[styles.actionText, { color: t.mutedForeground }]}>Cancel</Text>
            </Pressable>
            <Pressable
              onPress={() => {
                void handleSave();
              }}
              disabled={!canSave}
              style={styles.action}
            >
              <Text
                style={[
                  styles.actionText,
                  { color: canSave ? t.userMessage : `${t.userMessage}88` },
                ]}
              >
                {isSaving ? "Saving..." : isCreate ? "Create" : "Save"}
              </Text>
            </Pressable>
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.44)",
    justifyContent: "flex-end",
    padding: 12,
  },
  panel: {
    width: "100%",
    maxHeight: "92%",
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  header: {
    paddingHorizontal: 16,
    paddingTop: 16,
    paddingBottom: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 4,
  },
  title: {
    fontSize: 18,
    fontWeight: "600",
    letterSpacing: -0.3,
  },
  subtitle: {
    fontSize: 12,
    lineHeight: 18,
  },
  body: {
    flexGrow: 0,
  },
  bodyContent: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    gap: 8,
  },
  fieldLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
    marginTop: 6,
  },
  fieldHint: {
    fontSize: 12,
    lineHeight: 17,
  },
  input: {
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    fontSize: 14,
  },
  colorRow: {
    flexDirection: "row",
    gap: 10,
  },
  colorSwatch: {
    width: 26,
    height: 26,
    borderRadius: 13,
  },
  modelList: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    overflow: "hidden",
  },
  modelScroll: {
    maxHeight: 180,
  },
  modelRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  modelName: {
    flex: 1,
    fontSize: 13,
    fontWeight: "500",
  },
  modelProvider: {
    fontSize: 12,
  },
  choiceRow: {
    flexDirection: "row",
    gap: 8,
  },
  choice: {
    paddingHorizontal: 12,
    paddingVertical: 7,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
  },
  choiceText: {
    fontSize: 12,
    fontWeight: "600",
  },
  lifecycle: {
    marginTop: 14,
    paddingTop: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    justifyContent: "space-between",
  },
  lifecycleAction: {
    paddingVertical: 4,
  },
  lifecycleText: {
    fontSize: 13,
    fontWeight: "600",
  },
  actions: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  action: {
    paddingVertical: 4,
  },
  actionText: {
    fontSize: 13,
    fontWeight: "600",
  },
});
