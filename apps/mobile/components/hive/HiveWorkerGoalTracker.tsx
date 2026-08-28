import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  Check,
  FolderOpen,
  Pause,
  Play,
  Plus,
  RefreshCw,
  X,
} from "lucide-react-native";

import { DirectoryPicker } from "../DirectoryPicker";
import { useThemeContext } from "../../hooks/useTheme";
import type { HiveWorkerGoalState } from "./hooks/useHiveWorkerGoal";
import {
  confirmWorkerGoalCancellation,
  createWorkerGoalCancellationGuard,
  type WorkerGoalCancellationContext,
} from "./worker-goal-cancel-confirmation";

interface HiveWorkerGoalTrackerProps {
  state: HiveWorkerGoalState;
  bottom: number;
  onHeightChange: (height: number) => void;
}

function readableStatus(value: string): string {
  return value.replace(/_/g, " ");
}

export function HiveWorkerGoalTracker({
  state,
  bottom,
  onHeightChange,
}: HiveWorkerGoalTrackerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [pickerOpen, setPickerOpen] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [objective, setObjective] = useState("");
  const [successCriterion, setSuccessCriterion] = useState("");
  const [planSteps, setPlanSteps] = useState("");
  const [reviewReason, setReviewReason] = useState("");
  const [criterionReviews, setCriterionReviews] = useState<
    Record<string, { decision: "passed" | "waived" | null; evidence: string }>
  >({});
  const projection = state.projection;
  const workflow = projection?.workflow ?? null;
  const pendingAcceptance = projection?.pending_acceptance ?? null;
  const actions = new Set(projection?.allowed_actions ?? []);
  const cancelTargetKey = projection && workflow
    ? `${projection.worker_id}:${workflow.goal.id}`
    : null;
  const cancelContextRef = useRef<WorkerGoalCancellationContext>({
    targetKey: cancelTargetKey,
    canCancel: actions.has("cancel"),
    cancel: state.cancel,
  });
  cancelContextRef.current = {
    targetKey: cancelTargetKey,
    canCancel: actions.has("cancel"),
    cancel: state.cancel,
  };
  const cancelGuardRef = useRef<
    ReturnType<
      typeof createWorkerGoalCancellationGuard
    > | null
  >(null);
  if (!cancelGuardRef.current) {
    cancelGuardRef.current = createWorkerGoalCancellationGuard();
  }
  const cancelSurfaceMountedRef = useRef(false);

  useEffect(() => () => onHeightChange(0), [onHeightChange]);
  useEffect(() => {
    cancelSurfaceMountedRef.current = true;
    return () => {
      // A native Alert callback can outlive this keyed Worker/Goal surface.
      // Invalidate that callback before a different Worker becomes active.
      cancelSurfaceMountedRef.current = false;
    };
  }, []);
  useEffect(() => {
    setReviewReason("");
    setCriterionReviews({});
  }, [pendingAcceptance?.acceptance_run_id]);

  if (!projection && !state.error) return null;

  const completed =
    workflow?.steps.filter((step) =>
      step.status === "completed" || step.status === "skipped"
    ).length ?? 0;
  const total = workflow?.steps.length ?? 0;
  const runStatus = projection?.active_run?.run_status;
  const status = runStatus ?? workflow?.goal.status ??
    projection?.worker_status;

  const confirmCancel = () => {
    const requestedTargetKey = cancelContextRef.current.targetKey;
    confirmWorkerGoalCancellation({
      isWeb: Platform.OS === "web",
      confirmWeb: typeof window === "undefined"
        ? undefined
        : (message) => window.confirm(message),
      showNativeAlert: (alertTitle, alertMessage, buttons) => {
        Alert.alert(alertTitle, alertMessage, buttons);
      },
      onConfirm: () => {
        if (!cancelSurfaceMountedRef.current) return;
        cancelGuardRef.current?.attempt(
          requestedTargetKey,
          () => cancelContextRef.current,
        );
      },
    });
  };

  const createReady = title.trim().length > 0 &&
    objective.trim().length > 0 &&
    successCriterion.trim().length > 0 &&
    planSteps.trim().length > 0 &&
    planSteps.split("\n").filter((step) => step.trim().length > 0).length <= 12;
  const submitGoal = () => {
    if (!createReady || state.isSaving) return;
    void state.create({ title, objective, successCriterion, planSteps })
      .then(() => {
        setEditorOpen(false);
        setTitle("");
        setObjective("");
        setSuccessCriterion("");
        setPlanSteps("");
      })
      .catch(() => undefined);
  };
  const acceptanceReady = Boolean(
    pendingAcceptance && reviewReason.trim() &&
      pendingAcceptance.required_goal_criteria.every((criterion) => {
        const review = criterionReviews[criterion.criterion_id];
        return review?.decision && review.evidence.trim();
      }),
  );
  const acceptPending = () => {
    if (!pendingAcceptance || !acceptanceReady || state.isSaving) return;
    void state.accept({
      reason: reviewReason,
      criteria: pendingAcceptance.required_goal_criteria.map((criterion) => ({
        criterionId: criterion.criterion_id,
        decision: criterionReviews[criterion.criterion_id]!.decision!,
        evidence: criterionReviews[criterion.criterion_id]!.evidence,
      })),
    }).catch(() => undefined);
  };
  const rejectPending = () => {
    if (!pendingAcceptance || !reviewReason.trim() || state.isSaving) return;
    void state.reject(reviewReason).catch(() => undefined);
  };

  return (
    <>
      <View
        accessibilityLabel="Worker Goal tracker"
        onLayout={(event) =>
          onHeightChange(Math.ceil(event.nativeEvent.layout.height))}
        style={[
          styles.root,
          {
            bottom,
            backgroundColor: t.glass.background,
            borderColor: t.glass.border,
          },
        ]}
      >
        <View style={styles.headingRow}>
          <View style={styles.headingCopy}>
            <Text style={[styles.eyebrow, { color: t.mutedForeground }]}>
              WORKER GOAL
            </Text>
            <Text
              numberOfLines={1}
              style={[styles.title, { color: t.foreground }]}
            >
              {workflow?.goal.title ??
                (projection?.workspace.mode === "neutral"
                  ? "Choose a workspace"
                  : "Ready for a Goal")}
            </Text>
          </View>
          {state.isLoading || state.isSaving
            ? <ActivityIndicator size="small" color={t.primary} />
            : status
            ? (
              <Text style={[styles.status, { color: t.mutedForeground }]}>
                {readableStatus(status)}
              </Text>
            )
            : null}
        </View>

        {workflow?.goal.objective
          ? (
            <Text
              numberOfLines={2}
              style={[styles.objective, { color: t.mutedForeground }]}
            >
              {workflow.goal.objective}
            </Text>
          )
          : null}

        {total > 0
          ? (
            <Text style={[styles.progress, { color: t.mutedForeground }]}>
              {completed} of {total} plan steps complete
            </Text>
          )
          : null}

        {projection?.read_only_reason
          ? (
            <Text style={[styles.notice, { color: t.mutedForeground }]}>
              {projection.read_only_reason}
            </Text>
          )
          : null}
        {projection?.attention.map((message) => (
          <Text key={message} style={[styles.notice, { color: t.error }]}>
            {message}
          </Text>
        ))}
        {state.error
          ? (
            <Text style={[styles.notice, { color: t.error }]}>
              {state.error}
            </Text>
          )
          : null}
        {state.error && !projection
          ? (
            <View style={styles.actionRow}>
              <GoalAction
                label="Retry Goal"
                icon={<RefreshCw size={14} color={t.foreground} />}
                disabled={state.isLoading || state.isSaving}
                foreground={t.foreground}
                background={t.surface}
                onPress={state.refresh}
              />
            </View>
          )
          : null}

        {pendingAcceptance
          ? (
            <View
              accessibilityLabel="Review Worker Goal step"
              style={[styles.review, { borderColor: t.border }]}
            >
              <Text style={[styles.reviewTitle, { color: t.foreground }]}>
                Review step result
              </Text>
              <Text style={[styles.reviewStep, { color: t.mutedForeground }]}>
                {pendingAcceptance.step_description}
              </Text>
              <ScrollView
                keyboardShouldPersistTaps="handled"
                nestedScrollEnabled
                style={styles.reviewScroll}
              >
                <View
                  accessibilityLabel="Worker Goal source summary"
                  style={[styles.sourceSummary, { backgroundColor: t.surface }]}
                >
                  <Text style={[styles.sourceOutcome, { color: t.foreground }]}>
                    Observed{" "}
                    {readableStatus(pendingAcceptance.source_summary.outcome)}
                  </Text>
                  {pendingAcceptance.source_summary.effect.summary
                    ? (
                      <Text
                        style={[styles.sourceEffect, {
                          color: t.mutedForeground,
                        }]}
                      >
                        {pendingAcceptance.source_summary.effect.summary}
                      </Text>
                    )
                    : null}
                  <Text
                    style={[styles.sourceCounters, {
                      color: t.mutedForeground,
                    }]}
                  >
                    {pendingAcceptance.source_summary.counters.turns} turns ·
                    {" "}
                    {pendingAcceptance.source_summary.counters.provider_calls}
                    {" "}
                    provider calls ·{" "}
                    {pendingAcceptance.source_summary.counters.tool_calls}{" "}
                    tools ({pendingAcceptance.source_summary.counters
                      .failed_tool_calls} failed)
                  </Text>
                  {pendingAcceptance.source_summary.evidence.map((
                    evidence,
                    index,
                  ) => (
                    <View
                      key={`${evidence.kind}:${index}`}
                      style={styles.sourceEvidence}
                    >
                      <Text
                        style={[styles.sourceEvidenceKind, {
                          color: t.mutedForeground,
                        }]}
                      >
                        {readableStatus(evidence.kind)}
                      </Text>
                      <Text
                        style={[styles.sourceEvidenceText, {
                          color: t.foreground,
                        }]}
                      >
                        {evidence.summary}
                      </Text>
                    </View>
                  ))}
                </View>
                {pendingAcceptance.required_goal_criteria.map((criterion) => {
                  const review = criterionReviews[criterion.criterion_id] ?? {
                    decision: null,
                    evidence: "",
                  };
                  const update = (
                    next: Partial<typeof review>,
                  ) =>
                    setCriterionReviews((current) => ({
                      ...current,
                      [criterion.criterion_id]: { ...review, ...next },
                    }));
                  return (
                    <View
                      key={criterion.criterion_id}
                      style={styles.criterionReview}
                    >
                      <Text
                        style={[styles.criterionText, { color: t.foreground }]}
                      >
                        {criterion.description}
                      </Text>
                      <View style={styles.choiceRow}>
                        {(["passed", "waived"] as const).map((decision) => (
                          <Pressable
                            key={decision}
                            accessibilityLabel={`${decision} ${criterion.description}`}
                            accessibilityRole="button"
                            onPress={() => update({ decision })}
                            style={[
                              styles.choice,
                              {
                                borderColor: review.decision === decision
                                  ? t.primary
                                  : t.border,
                                backgroundColor: review.decision === decision
                                  ? `${t.primary}18`
                                  : t.surface,
                              },
                            ]}
                          >
                            <Text
                              style={[styles.choiceText, {
                                color: t.foreground,
                              }]}
                            >
                              {decision === "passed" ? "Passed" : "Waived"}
                            </Text>
                          </Pressable>
                        ))}
                      </View>
                      <TextInput
                        accessibilityLabel={`Evidence for ${criterion.description}`}
                        maxLength={4_000}
                        onChangeText={(evidence) => update({ evidence })}
                        placeholder="Concrete evidence"
                        placeholderTextColor={t.mutedForeground}
                        style={[styles.reviewInput, {
                          borderColor: t.border,
                          color: t.foreground,
                        }]}
                        value={review.evidence}
                      />
                    </View>
                  );
                })}
              </ScrollView>
              <TextInput
                accessibilityLabel="Worker Goal review reason"
                maxLength={4_000}
                multiline
                onChangeText={setReviewReason}
                placeholder="Why are you accepting or rejecting this result?"
                placeholderTextColor={t.mutedForeground}
                style={[styles.reviewInput, {
                  borderColor: t.border,
                  color: t.foreground,
                }]}
                value={reviewReason}
              />
              <View style={styles.actionRow}>
                <GoalAction
                  label="Accept result"
                  icon={<Check size={14} color={t.onAccent} />}
                  disabled={!acceptanceReady || state.isSaving}
                  foreground={t.onAccent}
                  background={t.userMessage}
                  onPress={acceptPending}
                />
                <GoalAction
                  label="Reject result"
                  icon={<X size={14} color={t.error} />}
                  disabled={!reviewReason.trim() || state.isSaving}
                  foreground={t.error}
                  background={`${t.error}14`}
                  onPress={rejectPending}
                />
              </View>
            </View>
          )
          : null}

        {projection && actions.size > 0
          ? (
            <View style={styles.actionRow}>
              {actions.has("create_goal")
                ? (
                  <GoalAction
                    label="Create Goal"
                    icon={<Plus size={14} color={t.onAccent} />}
                    disabled={state.isSaving}
                    foreground={t.onAccent}
                    background={t.userMessage}
                    onPress={() => setEditorOpen(true)}
                  />
                )
                : null}
              {actions.has("set_workspace")
                ? (
                  <GoalAction
                    label={projection.workspace.mode === "neutral"
                      ? "Choose workspace"
                      : "Change workspace"}
                    icon={<FolderOpen size={14} color={t.onAccent} />}
                    disabled={state.isSaving}
                    foreground={t.onAccent}
                    background={t.userMessage}
                    onPress={() => setPickerOpen(true)}
                  />
                )
                : null}
              {actions.has("approve_plan")
                ? (
                  <GoalAction
                    label="Approve plan"
                    icon={<Check size={14} color={t.onAccent} />}
                    disabled={state.isSaving}
                    foreground={t.onAccent}
                    background={t.userMessage}
                    onPress={() => {
                      void state.approve().catch(() => undefined);
                    }}
                  />
                )
                : null}
              {actions.has("activate")
                ? (
                  <GoalAction
                    label={workflow?.goal.status === "paused"
                      ? "Resume"
                      : "Start"}
                    icon={<Play size={14} color={t.onAccent} />}
                    disabled={state.isSaving}
                    foreground={t.onAccent}
                    background={t.userMessage}
                    onPress={() => {
                      void state.activate().catch(() => undefined);
                    }}
                  />
                )
                : null}
              {actions.has("pause")
                ? (
                  <GoalAction
                    label="Pause"
                    icon={<Pause size={14} color={t.foreground} />}
                    disabled={state.isSaving}
                    foreground={t.foreground}
                    background={t.surface}
                    onPress={() => {
                      void state.pause().catch(() => undefined);
                    }}
                  />
                )
                : null}
              {actions.has("cancel")
                ? (
                  <GoalAction
                    label="Cancel"
                    icon={<X size={14} color={t.error} />}
                    disabled={state.isSaving}
                    foreground={t.error}
                    background={`${t.error}14`}
                    onPress={confirmCancel}
                  />
                )
                : null}
            </View>
          )
          : null}
      </View>
      <DirectoryPicker
        visible={pickerOpen}
        initialPath={projection?.workspace.project_dir ?? undefined}
        onClose={() => setPickerOpen(false)}
        onSelect={(path) => {
          void state.setWorkspace(path)
            .then(() => setPickerOpen(false))
            .catch(() => undefined);
        }}
      />
      <Modal
        animationType="fade"
        transparent
        visible={editorOpen}
        onRequestClose={() => {
          if (!state.isSaving) setEditorOpen(false);
        }}
      >
        <View style={styles.modalBackdrop}>
          <KeyboardAvoidingView
            behavior={Platform.OS === "ios" ? "padding" : "height"}
            style={styles.keyboardSafe}
          >
            <View
              accessibilityLabel="Create Worker Goal"
              style={[
                styles.editor,
                { backgroundColor: t.background, borderColor: t.border },
              ]}
            >
              <ScrollView
                contentContainerStyle={styles.editorContent}
                keyboardShouldPersistTaps="handled"
                showsVerticalScrollIndicator={false}
              >
                <View style={styles.editorHeading}>
                  <View style={styles.headingCopy}>
                    <Text style={[styles.editorTitle, { color: t.foreground }]}>
                      Create Worker Goal
                    </Text>
                    <Text
                      style={[styles.editorHint, { color: t.mutedForeground }]}
                    >
                      Define the outcome and up to 12 plan steps.
                    </Text>
                  </View>
                  <Pressable
                    accessibilityLabel="Close Goal editor"
                    accessibilityRole="button"
                    disabled={state.isSaving}
                    onPress={() => setEditorOpen(false)}
                    hitSlop={10}
                  >
                    <X size={18} color={t.mutedForeground} />
                  </Pressable>
                </View>
                <GoalField
                  label="Title"
                  value={title}
                  onChangeText={setTitle}
                  placeholder="Ship the Worker Goal bridge"
                  color={t.foreground}
                  mutedColor={t.mutedForeground}
                  borderColor={t.border}
                />
                <GoalField
                  label="Objective"
                  value={objective}
                  onChangeText={setObjective}
                  placeholder="What must be true when this Goal is done?"
                  color={t.foreground}
                  mutedColor={t.mutedForeground}
                  borderColor={t.border}
                  multiline
                />
                <GoalField
                  label="Success criterion"
                  value={successCriterion}
                  onChangeText={setSuccessCriterion}
                  placeholder="A concrete check that proves completion"
                  color={t.foreground}
                  mutedColor={t.mutedForeground}
                  borderColor={t.border}
                  multiline
                />
                <GoalField
                  label="Plan steps (one per line)"
                  value={planSteps}
                  onChangeText={setPlanSteps}
                  placeholder={"Inspect the current state\nImplement the bounded change\nRun focused validation"}
                  color={t.foreground}
                  mutedColor={t.mutedForeground}
                  borderColor={t.border}
                  multiline
                />
                {state.error
                  ? (
                    <Text style={[styles.editorError, { color: t.error }]}>
                      {state.error}
                    </Text>
                  )
                  : null}
                <Pressable
                  accessibilityLabel="Save Worker Goal"
                  accessibilityRole="button"
                  disabled={!createReady || state.isSaving}
                  onPress={submitGoal}
                  style={({ pressed }) => [
                    styles.saveGoal,
                    {
                      backgroundColor: t.userMessage,
                      opacity: !createReady || state.isSaving
                        ? 0.45
                        : pressed
                        ? 0.72
                        : 1,
                    },
                  ]}
                >
                  {state.isSaving
                    ? <ActivityIndicator size="small" color={t.onAccent} />
                    : <Plus size={15} color={t.onAccent} />}
                  <Text style={[styles.actionText, { color: t.onAccent }]}>
                    Create Goal and plan
                  </Text>
                </Pressable>
              </ScrollView>
            </View>
          </KeyboardAvoidingView>
        </View>
      </Modal>
    </>
  );
}

interface GoalFieldProps {
  label: string;
  value: string;
  onChangeText: (value: string) => void;
  placeholder: string;
  color: string;
  mutedColor: string;
  borderColor: string;
  multiline?: boolean;
}

function GoalField({
  label,
  value,
  onChangeText,
  placeholder,
  color,
  mutedColor,
  borderColor,
  multiline = false,
}: GoalFieldProps) {
  return (
    <View style={styles.field}>
      <Text style={[styles.fieldLabel, { color: mutedColor }]}>{label}</Text>
      <TextInput
        accessibilityLabel={label}
        editable
        maxLength={4_000}
        multiline={multiline}
        onChangeText={onChangeText}
        placeholder={placeholder}
        placeholderTextColor={mutedColor}
        style={[
          styles.fieldInput,
          multiline && styles.fieldInputMultiline,
          { borderColor, color },
        ]}
        value={value}
      />
    </View>
  );
}

interface GoalActionProps {
  label: string;
  icon: ReactNode;
  disabled: boolean;
  foreground: string;
  background: string;
  onPress: () => void;
}

function GoalAction({
  label,
  icon,
  disabled,
  foreground,
  background,
  onPress,
}: GoalActionProps) {
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.action,
        {
          backgroundColor: background,
          opacity: disabled ? 0.45 : pressed ? 0.72 : 1,
        },
      ]}
    >
      {icon}
      <Text style={[styles.actionText, { color: foreground }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  root: {
    position: "absolute",
    left: 12,
    right: 12,
    zIndex: 18,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 16,
    paddingHorizontal: 13,
    paddingVertical: 11,
    gap: 5,
  },
  headingRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  headingCopy: {
    flex: 1,
    minWidth: 0,
  },
  eyebrow: {
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  title: {
    marginTop: 1,
    fontSize: 14,
    fontWeight: "700",
  },
  status: {
    fontSize: 10,
    fontWeight: "700",
    textTransform: "capitalize",
  },
  objective: {
    fontSize: 11,
    lineHeight: 15,
  },
  progress: {
    fontSize: 10,
    fontWeight: "600",
  },
  notice: {
    fontSize: 10,
    lineHeight: 14,
  },
  review: {
    marginTop: 4,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    padding: 10,
    gap: 8,
  },
  reviewTitle: {
    fontSize: 12,
    fontWeight: "700",
  },
  reviewScroll: {
    maxHeight: 240,
  },
  reviewStep: {
    fontSize: 11,
    lineHeight: 15,
  },
  sourceSummary: {
    borderRadius: 9,
    padding: 9,
    gap: 5,
    marginBottom: 8,
  },
  sourceOutcome: {
    fontSize: 11,
    fontWeight: "700",
    textTransform: "capitalize",
  },
  sourceEffect: {
    fontSize: 10,
    lineHeight: 14,
  },
  sourceCounters: {
    fontSize: 9,
    lineHeight: 13,
  },
  sourceEvidence: {
    gap: 1,
  },
  sourceEvidenceKind: {
    fontSize: 8,
    fontWeight: "800",
    letterSpacing: 0.4,
    textTransform: "uppercase",
  },
  sourceEvidenceText: {
    fontSize: 10,
    lineHeight: 14,
  },
  criterionReview: {
    gap: 6,
  },
  criterionText: {
    fontSize: 11,
    fontWeight: "600",
  },
  choiceRow: {
    flexDirection: "row",
    gap: 6,
  },
  choice: {
    minHeight: 28,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 9,
    paddingHorizontal: 9,
    alignItems: "center",
    justifyContent: "center",
  },
  choiceText: {
    fontSize: 10,
    fontWeight: "700",
  },
  reviewInput: {
    minHeight: 38,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 9,
    paddingHorizontal: 10,
    paddingVertical: 8,
    fontSize: 11,
  },
  actionRow: {
    marginTop: 4,
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 7,
  },
  action: {
    minHeight: 30,
    borderRadius: 10,
    paddingHorizontal: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 5,
  },
  actionText: {
    fontSize: 11,
    fontWeight: "700",
  },
  modalBackdrop: {
    flex: 1,
    backgroundColor: "rgba(0, 0, 0, 0.48)",
  },
  keyboardSafe: {
    flex: 1,
    justifyContent: "center",
    padding: 20,
  },
  editor: {
    maxHeight: "88%",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 18,
    overflow: "hidden",
  },
  editorContent: {
    padding: 16,
    gap: 12,
  },
  editorHeading: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  editorTitle: {
    fontSize: 17,
    fontWeight: "700",
  },
  editorHint: {
    marginTop: 2,
    fontSize: 11,
    lineHeight: 15,
  },
  field: {
    gap: 5,
  },
  fieldLabel: {
    fontSize: 10,
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  fieldInput: {
    minHeight: 40,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 11,
    paddingVertical: 9,
    fontSize: 13,
  },
  fieldInputMultiline: {
    minHeight: 62,
    textAlignVertical: "top",
  },
  editorError: {
    fontSize: 11,
    lineHeight: 15,
  },
  saveGoal: {
    minHeight: 40,
    borderRadius: 11,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
  },
});
