import { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import {
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Circle,
  ListChecks,
  Target,
} from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { AdaptiveMaterial } from "../ui/AdaptiveMaterial";
import { useThemeContext } from "../../hooks/useTheme";
import { usePlanStore, useSessionStore } from "../../hooks/useStores";
import type { SessionType } from "@mitsuro/api";

interface PlanTrackerProps {
  sessionType?: SessionType;
  onHeightChange?: (height: number) => void;
}

export function PlanTracker({
  sessionType = "chat",
  onHeightChange,
}: PlanTrackerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const lastReportedHeightRef = useRef(0);
  const [commandPending, setCommandPending] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [goalExpanded, setGoalExpanded] = useState(false);
  const [planExpanded, setPlanExpanded] = useState(false);

  const planItems = usePlanStore((s) => s.items, sessionType);
  const workflow = usePlanStore((s) => s.workflow, sessionType);
  const pendingRevision = usePlanStore((s) => s.pendingRevision, sessionType);
  const items = planItems ?? [];
  const goalAvailable = workflow != null;
  const planAvailable = items.length > 0 || workflow?.plan_revision != null;
  const trackerAvailable = goalAvailable || planAvailable;
  const goalId = workflow?.goal.id ?? null;
  const lastGoalIdRef = useRef(goalId);
  const executeWorkflowCommand = useSessionStore(
    (s) => s.executeWorkflowCommand,
    sessionType,
  );

  useEffect(() => {
    if (trackerAvailable || lastReportedHeightRef.current === 0) return;
    lastReportedHeightRef.current = 0;
    onHeightChange?.(0);
  }, [onHeightChange, trackerAvailable]);

  useEffect(() => {
    if (lastGoalIdRef.current === goalId) return;
    lastGoalIdRef.current = goalId;
    setGoalExpanded(false);
    setPlanExpanded(false);
  }, [goalId]);

  const handleLayout = useCallback(({ nativeEvent }: any) => {
    const nextHeight = Math.ceil(nativeEvent.layout.height);
    if (lastReportedHeightRef.current === nextHeight) return;
    lastReportedHeightRef.current = nextHeight;
    onHeightChange?.(nextHeight);
  }, [onHeightChange]);

  if (!trackerAvailable) return null;

  const completed = items.filter((item) => item.completed).length;
  const total = items.length;
  const sectionControlCount = Number(goalAvailable) + Number(planAvailable);
  const allowed = new Set(workflow?.allowed_actions ?? []);
  const commandAction = allowed.has("approve_plan")
    ? "approve_plan"
    : allowed.has("activate_goal")
    ? "activate_goal"
    : allowed.has("pause_goal")
    ? "pause_goal"
    : allowed.has("resume_goal")
    ? "resume_goal"
    : null;

  const runCommand = async () => {
    if (!workflow || !commandAction || commandPending) return;
    setCommandPending(true);
    setCommandError(null);
    try {
      const base = {
        operation_id: `mobile:${commandAction}:${Date.now()}`,
        goal_id: workflow.goal.id,
        expected_revision: workflow.aggregate_revision,
      };
      if (commandAction === "approve_plan") {
        if (!workflow.plan_revision) return;
        const approved = await executeWorkflowCommand({
          action: commandAction,
          ...base,
          plan_revision_id: workflow.plan_revision.id,
        });
        const startAction = approved.snapshot.allowed_actions.includes(
            "activate_goal",
          )
          ? "activate_goal"
          : approved.snapshot.allowed_actions.includes("resume_goal")
          ? "resume_goal"
          : null;
        if (!startAction) {
          throw new Error(
            "Plan approved, but the goal is not ready to start yet.",
          );
        }
        await executeWorkflowCommand({
          action: startAction,
          operation_id: `mobile:${startAction}:${Date.now()}`,
          goal_id: approved.snapshot.goal.id,
          expected_revision: approved.snapshot.aggregate_revision,
        });
      } else if (commandAction === "pause_goal") {
        await executeWorkflowCommand({
          action: commandAction,
          ...base,
          reason: "paused_from_mobile",
        });
      } else {
        await executeWorkflowCommand({ action: commandAction, ...base });
      }
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch (error) {
      setCommandError(
        error instanceof Error ? error.message : "Could not update the plan.",
      );
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
    } finally {
      setCommandPending(false);
    }
  };

  const setSectionExpanded = (
    section: "goal" | "plan",
    expanded: boolean,
  ) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (section === "goal") setGoalExpanded(expanded);
    else setPlanExpanded(expanded);
  };

  const renderMaterial = () => (
    <AdaptiveMaterial tone="elevated" borderRadius={14} />
  );

  return (
    <View style={styles.trackerStack} onLayout={handleLayout}>
      {goalAvailable && goalExpanded
        ? (
          <View style={[styles.panel, { borderColor: t.glass.border }]}>
            {renderMaterial()}
            <View style={styles.goalSection}>
              <View style={styles.sectionHeading}>
                <Target size={15} color={t.thinking} strokeWidth={1.8} />
                <Text style={[styles.sectionLabel, { color: t.thinking }]}>
                  GOAL
                </Text>
                <Text
                  style={[styles.goalStatus, { color: t.mutedForeground }]}
                >
                  {workflow.goal.status.replace("_", " ")}
                  {pendingRevision ? " · syncing" : ""}
                </Text>
                <View style={styles.headingSpacer} />
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Collapse goal"
                  onPress={() => setSectionExpanded("goal", false)}
                  hitSlop={8}
                  style={styles.iconButton}
                >
                  <ChevronDown
                    size={16}
                    color={t.mutedForeground}
                    strokeWidth={2}
                  />
                </Pressable>
              </View>
              <Text
                style={[styles.goalTitle, { color: t.foreground }]}
                numberOfLines={2}
              >
                {workflow.goal.title}
              </Text>
              {workflow.goal.objective
                ? (
                  <Text
                    style={[styles.objective, { color: t.mutedForeground }]}
                    numberOfLines={2}
                  >
                    {workflow.goal.objective}
                  </Text>
                )
                : null}
              {workflow.criteria.length > 0
                ? (
                  <View style={styles.criteria}>
                    <Text
                      style={[styles.criteriaLabel, {
                        color: t.mutedForeground,
                      }]}
                    >
                      SUCCESS CRITERIA
                    </Text>
                    {workflow.criteria.slice(0, 3).map((criterion) => (
                      <View key={criterion.id} style={styles.criterion}>
                        <Circle
                          size={12}
                          color={t.mutedForeground}
                          strokeWidth={1.5}
                        />
                        <Text
                          style={[styles.criterionText, {
                            color: t.foreground,
                          }]}
                          numberOfLines={1}
                        >
                          {criterion.description}
                        </Text>
                      </View>
                    ))}
                  </View>
                )
                : null}
            </View>
          </View>
        )
        : null}

      {planAvailable && planExpanded
        ? (
          <View style={[styles.panel, { borderColor: t.glass.border }]}>
            {renderMaterial()}
            <View style={styles.planHeading}>
              <ListChecks size={15} color={t.thinking} strokeWidth={1.8} />
              <View style={styles.headerCopy}>
                <Text style={[styles.sectionLabel, { color: t.thinking }]}>
                  PLAN
                </Text>
                <Text
                  style={[styles.planTitle, { color: t.foreground }]}
                  numberOfLines={1}
                >
                  {workflow?.plan_revision?.title ?? "Plan"}
                </Text>
              </View>
              <Text
                style={[styles.headerCount, { color: t.mutedForeground }]}
              >
                {completed}/{total}
              </Text>
              {commandAction
                ? (
                  <Pressable
                    accessibilityRole="button"
                    disabled={commandPending}
                    onPress={runCommand}
                    style={[
                      styles.commandButton,
                      {
                        borderColor: t.border,
                        opacity: commandPending ? 0.5 : 1,
                      },
                    ]}
                  >
                    <Text
                      style={[styles.commandButtonText, {
                        color: t.foreground,
                      }]}
                    >
                      {commandPending
                        ? "Working…"
                        : commandAction === "approve_plan"
                        ? "Start"
                        : commandAction === "activate_goal"
                        ? "Start"
                        : commandAction === "pause_goal"
                        ? "Pause"
                        : "Resume"}
                    </Text>
                  </Pressable>
                )
                : null}
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Collapse plan"
                onPress={() => setSectionExpanded("plan", false)}
                hitSlop={8}
                style={styles.iconButton}
              >
                <ChevronDown
                  size={16}
                  color={t.mutedForeground}
                  strokeWidth={2}
                />
              </Pressable>
            </View>
            {commandError
              ? (
                <Text style={[styles.commandError, { color: t.error }]}>
                  {commandError}
                </Text>
              )
              : null}
            <ScrollView
              style={styles.itemsScroll}
              contentContainerStyle={styles.items}
              nestedScrollEnabled
              showsVerticalScrollIndicator={false}
            >
              {items.map((item) => (
                <View key={item.id} style={styles.item}>
                  {item.completed
                    ? (
                      <CheckCircle2
                        size={16}
                        color={t.success}
                        strokeWidth={2}
                      />
                    )
                    : (
                      <Circle
                        size={16}
                        color={t.mutedForeground}
                        strokeWidth={1.5}
                      />
                    )}
                  <Text
                    style={[
                      styles.itemText,
                      {
                        color: item.completed
                          ? t.mutedForeground
                          : t.foreground,
                      },
                      item.completed && styles.itemCompleted,
                    ]}
                    numberOfLines={2}
                  >
                    {item.content}
                  </Text>
                </View>
              ))}
            </ScrollView>
            <View
              style={[
                styles.progressTrack,
                { backgroundColor: `${t.border}44` },
              ]}
            >
              <View
                style={[
                  styles.progressFill,
                  {
                    width: `${(completed / Math.max(total, 1)) * 100}%`,
                    backgroundColor: completed === total
                      ? t.success
                      : t.thinking,
                  },
                ]}
              />
            </View>
          </View>
        )
        : null}

      <View style={styles.controlRow}>
        {goalAvailable
          ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={goalExpanded
                ? "Collapse goal"
                : "Expand goal"}
              accessibilityState={{ expanded: goalExpanded }}
              onPress={() => setSectionExpanded("goal", !goalExpanded)}
              style={[
                styles.collapsedChip,
                sectionControlCount > 1
                  ? styles.collapsedChipShared
                  : styles.collapsedChipSolo,
                {
                  borderColor: t.border,
                  backgroundColor: t.surfaceOverlayElevated,
                },
              ]}
            >
              <Target size={14} color={t.thinking} strokeWidth={1.8} />
              <Text style={[styles.sectionLabel, { color: t.thinking }]}>
                GOAL
              </Text>
              <Text
                style={[styles.collapsedChipText, { color: t.foreground }]}
                numberOfLines={1}
              >
                {workflow.goal.title}
              </Text>
              {goalExpanded
                ? (
                  <ChevronDown
                    size={14}
                    color={t.mutedForeground}
                    strokeWidth={2}
                  />
                )
                : (
                  <ChevronUp
                    size={14}
                    color={t.mutedForeground}
                    strokeWidth={2}
                  />
                )}
            </Pressable>
          )
          : null}

        {planAvailable
          ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={planExpanded
                ? "Collapse plan"
                : "Expand plan"}
              accessibilityState={{ expanded: planExpanded }}
              onPress={() => setSectionExpanded("plan", !planExpanded)}
              style={[
                styles.collapsedChip,
                sectionControlCount > 1
                  ? styles.collapsedChipShared
                  : styles.collapsedChipSolo,
                {
                  borderColor: t.border,
                  backgroundColor: t.surfaceOverlayElevated,
                },
              ]}
            >
              <ListChecks size={14} color={t.thinking} strokeWidth={1.8} />
              <Text style={[styles.sectionLabel, { color: t.thinking }]}>
                PLAN
              </Text>
              <Text
                style={[styles.collapsedChipText, { color: t.foreground }]}
                numberOfLines={1}
              >
                {workflow?.plan_revision?.title ?? "Plan"}
              </Text>
              <Text style={[styles.headerCount, { color: t.mutedForeground }]}>
                {completed}/{total}
              </Text>
              {planExpanded
                ? (
                  <ChevronDown
                    size={14}
                    color={t.mutedForeground}
                    strokeWidth={2}
                  />
                )
                : (
                  <ChevronUp
                    size={14}
                    color={t.mutedForeground}
                    strokeWidth={2}
                  />
                )}
            </Pressable>
          )
          : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  trackerStack: {
    width: "100%",
    gap: 6,
  },
  controlRow: {
    width: "100%",
    flexDirection: "row",
    justifyContent: "center",
    alignItems: "stretch",
    gap: 6,
  },
  panel: {
    width: "100%",
    borderRadius: 14,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
  },
  collapsedChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  collapsedChipShared: { flex: 1, minWidth: 0, maxWidth: "49%" },
  collapsedChipSolo: { flexShrink: 1, maxWidth: "86%" },
  collapsedChipText: {
    flexShrink: 1,
    fontSize: 13,
    fontWeight: "600",
  },
  goalSection: {
    paddingHorizontal: 12,
    paddingTop: 9,
    paddingBottom: 10,
  },
  sectionHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  planHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingTop: 9,
  },
  sectionLabel: { fontSize: 9, fontWeight: "800", letterSpacing: 0.8 },
  goalTitle: { marginTop: 5, fontSize: 14, fontWeight: "600", lineHeight: 18 },
  planTitle: { marginTop: 1, fontSize: 12, fontWeight: "600" },
  headerCopy: { flex: 1, minWidth: 0 },
  goalStatus: { fontSize: 10, textTransform: "capitalize" },
  headingSpacer: { flex: 1 },
  headerCount: { fontSize: 12, fontWeight: "500" },
  commandButton: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 9,
    paddingVertical: 5,
  },
  commandButtonText: { fontSize: 11, fontWeight: "600" },
  iconButton: {
    width: 28,
    height: 28,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
  },
  itemsScroll: { maxHeight: 200 },
  items: { paddingHorizontal: 10, paddingVertical: 7, gap: 6 },
  commandError: {
    paddingHorizontal: 12,
    paddingTop: 6,
    fontSize: 11,
    lineHeight: 15,
  },
  objective: { marginTop: 4, fontSize: 12, lineHeight: 17 },
  criteria: { marginTop: 8, gap: 5 },
  criteriaLabel: { fontSize: 9, fontWeight: "700", letterSpacing: 0.7 },
  criterion: { flexDirection: "row", alignItems: "center", gap: 7 },
  criterionText: { flex: 1, fontSize: 11, lineHeight: 15 },
  item: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 8,
    paddingVertical: 4,
    paddingHorizontal: 4,
  },
  itemText: { flex: 1, fontSize: 13, lineHeight: 18 },
  itemCompleted: { textDecorationLine: "line-through", opacity: 0.6 },
  progressTrack: {
    height: 3,
    marginHorizontal: 14,
    marginBottom: 10,
    borderRadius: 1.5,
    overflow: "hidden",
  },
  progressFill: { height: "100%", borderRadius: 1.5 },
});
