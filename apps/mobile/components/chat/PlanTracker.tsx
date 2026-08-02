import { useCallback, useEffect, useRef, useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import {
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Circle,
  ListChecks,
  Minimize2,
} from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { BlurView } from "../../platform/blur";
import { useThemeContext } from "../../hooks/useTheme";
import { usePlanStore, useSessionStore } from "../../hooks/useStores";
import { useBreakpoint } from "../../hooks/useBreakpoint";
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
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const isDark = theme.scheme === "dark";
  const lastReportedHeightRef = useRef(0);
  const [commandPending, setCommandPending] = useState(false);
  const [isExpanded, setIsExpanded] = useState(true);

  const planItems = usePlanStore((s) => s.items, sessionType);
  const workflow = usePlanStore((s) => s.workflow, sessionType);
  const pendingRevision = usePlanStore((s) => s.pendingRevision, sessionType);
  const items = planItems ?? [];
  const isVisible = usePlanStore((s) => s.isVisible, sessionType);
  const setVisible = usePlanStore((s) => s.setVisible, sessionType);
  const trackerAvailable = items.length > 0;
  const executeWorkflowCommand = useSessionStore(
    (s) => s.executeWorkflowCommand,
    sessionType,
  );

  const planIdentity = workflow?.goal.id ?? items.map((item) => item.id).join("|");
  // Preserve the user's expanded/collapsed choice across plan identity updates.
  // Auto-forcing expand on every identity tick reflows the transcript and feels
  // like plan mode is taking over the screen.

  useEffect(() => {
    if (trackerAvailable || lastReportedHeightRef.current === 0) return;
    lastReportedHeightRef.current = 0;
    onHeightChange?.(0);
  }, [onHeightChange, trackerAvailable]);

  const handleLayout = useCallback(({ nativeEvent }: any) => {
    const nextHeight = Math.ceil(nativeEvent.layout.height);
    if (lastReportedHeightRef.current === nextHeight) return;
    lastReportedHeightRef.current = nextHeight;
    onHeightChange?.(nextHeight);
  }, [onHeightChange]);

  if (!trackerAvailable) return null;

  const completed = items.filter((i) => i.completed).length;
  const total = items.length;
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
    try {
      const base = {
        operation_id: `mobile:${commandAction}:${Date.now()}`,
        goal_id: workflow.goal.id,
        expected_revision: workflow.aggregate_revision,
      };
      if (commandAction === "approve_plan") {
        if (!workflow.plan_revision) return;
        await executeWorkflowCommand({
          action: commandAction,
          ...base,
          plan_revision_id: workflow.plan_revision.id,
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
    } finally {
      setCommandPending(false);
    }
  };

  const toggleExpanded = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setIsExpanded((current) => !current);
  };

  if (!isVisible) {
    return (
      <View
        style={[
          styles.container,
          isDesktop ? styles.containerDesktop : styles.containerMobile,
          styles.collapsedChipWrap,
        ]}
        onLayout={handleLayout}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Show plan"
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setVisible(true);
            setIsExpanded(true);
          }}
          style={[
            styles.collapsedChip,
            {
              borderColor: t.border,
              backgroundColor: t.surfaceOverlayElevated,
            },
          ]}
        >
          <ListChecks size={14} color={t.thinking} strokeWidth={1.8} />
          <Text style={[styles.collapsedChipText, { color: t.foreground }]} numberOfLines={1}>
            {workflow?.goal.title ?? "Plan"}
          </Text>
          <Text style={[styles.headerCount, { color: t.mutedForeground }]}>
            {completed}/{total}
          </Text>
          <ChevronDown size={14} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>
      </View>
    );
  }

  return (
    <View
      style={[styles.container, isDesktop ? styles.containerDesktop : styles.containerMobile]}
      onLayout={handleLayout}
    >
      <BlurView
        intensity={30}
        tint={isDark ? "systemMaterialDark" : "systemMaterialLight"}
        style={StyleSheet.absoluteFill}
      />
      <View
        style={[
          StyleSheet.absoluteFill,
          { backgroundColor: t.surfaceOverlay },
        ]}
      />

      <View style={[styles.header, { borderBottomColor: isExpanded ? t.border : "transparent" }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={isExpanded ? "Collapse plan" : "Expand plan"}
          onPress={toggleExpanded}
          style={styles.headerMain}
        >
          <ListChecks size={16} color={t.thinking} strokeWidth={1.8} />
          <View style={styles.headerCopy}>
            <Text style={[styles.headerTitle, { color: t.foreground }]} numberOfLines={1}>
              {workflow?.goal.title ?? "Plan"}
            </Text>
            {workflow ? (
              <Text style={[styles.goalStatus, { color: t.mutedForeground }]}>
                {workflow.goal.status.replace("_", " ")}
                {pendingRevision ? " · syncing" : ""}
              </Text>
            ) : null}
          </View>
          <Text style={[styles.headerCount, { color: t.mutedForeground }]}>
            {completed}/{total}
          </Text>
          {isExpanded ? (
            <ChevronUp size={16} color={t.mutedForeground} strokeWidth={2} />
          ) : (
            <ChevronDown size={16} color={t.mutedForeground} strokeWidth={2} />
          )}
        </Pressable>

        {commandAction ? (
          <Pressable
            accessibilityRole="button"
            disabled={commandPending}
            onPress={runCommand}
            style={[
              styles.commandButton,
              { borderColor: t.border, opacity: commandPending ? 0.5 : 1 },
            ]}
          >
            <Text style={[styles.commandButtonText, { color: t.foreground }]}>
              {commandPending
                ? "Working…"
                : commandAction === "approve_plan"
                  ? "Approve"
                  : commandAction === "activate_goal"
                    ? "Start"
                    : commandAction === "pause_goal"
                      ? "Pause"
                      : "Resume"}
            </Text>
          </Pressable>
        ) : null}

        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Hide plan"
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setIsExpanded(false);
            setVisible(false);
          }}
          hitSlop={8}
          style={styles.iconButton}
        >
          <Minimize2 size={15} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>
      </View>

      {isExpanded ? (
        <>
          {workflow?.goal.objective ? (
            <Text style={[styles.objective, { color: t.mutedForeground }]} numberOfLines={2}>
              {workflow.goal.objective}
            </Text>
          ) : null}
          <View style={styles.items}>
            {items.map((item) => (
              <Pressable
                key={item.id}
                onPress={() => {
                  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                }}
                style={styles.item}
              >
                {item.completed ? (
                  <CheckCircle2 size={16} color={t.success} strokeWidth={2} />
                ) : (
                  <Circle size={16} color={t.mutedForeground} strokeWidth={1.5} />
                )}
                <Text
                  style={[
                    styles.itemText,
                    { color: item.completed ? t.mutedForeground : t.foreground },
                    item.completed && styles.itemCompleted,
                  ]}
                  numberOfLines={2}
                >
                  {item.content}
                </Text>
              </Pressable>
            ))}
          </View>
          <View style={[styles.progressTrack, { backgroundColor: `${t.border}44` }]}>
            <View
              style={[
                styles.progressFill,
                {
                  width: `${(completed / Math.max(total, 1)) * 100}%`,
                  backgroundColor: completed === total ? t.success : t.thinking,
                },
              ]}
            />
          </View>
        </>
      ) : (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Expand plan details"
          onPress={toggleExpanded}
          style={styles.collapsedSummary}
        >
          <Text style={[styles.collapsedSummaryText, { color: t.mutedForeground }]} numberOfLines={1}>
            {completed === total
              ? "All steps complete"
              : `${total - completed} remaining · tap to expand`}
          </Text>
        </Pressable>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    borderRadius: 14,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.08)",
  },
  containerMobile: {
    position: "absolute",
    top: 8,
    left: 12,
    right: 12,
    zIndex: 50,
  },
  containerDesktop: {
    marginHorizontal: 12,
    marginBottom: 8,
  },
  collapsedChipWrap: {
    borderWidth: 0,
    backgroundColor: "transparent",
    overflow: "visible",
  },
  collapsedChip: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  collapsedChipText: {
    flexShrink: 1,
    fontSize: 13,
    fontWeight: "600",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerMain: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  headerTitle: { fontSize: 14, fontWeight: "600" },
  headerCopy: { flex: 1, minWidth: 0 },
  goalStatus: { marginTop: 1, fontSize: 11, textTransform: "capitalize" },
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
  items: { padding: 10, gap: 6, maxHeight: 200 },
  objective: { paddingHorizontal: 14, paddingTop: 9, fontSize: 12, lineHeight: 17 },
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
  collapsedSummary: { paddingHorizontal: 14, paddingBottom: 10 },
  collapsedSummaryText: { fontSize: 12, lineHeight: 16 },
});
