import { memo, useMemo } from "react";
import { useRouter } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Check, ChevronRight, Circle, Users, X } from "lucide-react-native";

import type { ToolCall } from "@mitsuro/api";
import {
  createDelegatedArtifactState,
  resolveDelegatedKind,
} from "@mitsuro/state";
import { useThemeContext } from "../../hooks/useTheme";
import * as Haptics from "../../platform/haptics";

interface AgentTeamCardProps {
  toolCall: ToolCall;
  sessionId?: string | null;
}

const ACTIVE_GROUP_STATES = new Set([
  "created",
  "queued",
  "running",
  "ready_for_parent",
  "synthesizing",
]);

function stateLabel(state: string | undefined) {
  if (state === "degraded") return "partial";
  return (state || "pending").replaceAll("_", " ");
}

export const AgentTeamCard = memo(function AgentTeamCard({
  toolCall,
  sessionId,
}: AgentTeamCardProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const router = useRouter();
  const delegated = useMemo(() => {
    const kind = resolveDelegatedKind(toolCall.name, toolCall.arguments);
    const seeded = kind
      ? createDelegatedArtifactState(kind, toolCall.arguments)
      : undefined;
    if (!toolCall.delegated) return seeded;
    if (!seeded || seeded.agents.length <= 1) return toolCall.delegated;

    // Streaming arguments can first create the legacy single `main` row and
    // then reveal the declared task graph. Never count that transient shell as
    // a third child; durable task IDs replace all declared rows once admitted.
    const hasRuntimeAgents = toolCall.delegated.agents.some(
      (agent) =>
        agent.taskId !== "main" && !agent.taskId.startsWith("declared:"),
    );
    const agents = toolCall.delegated.agents.filter((agent) => {
      if (agent.taskId === "main") return false;
      if (hasRuntimeAgents && agent.taskId.startsWith("declared:"))
        return false;
      return true;
    });
    if (agents.length === toolCall.delegated.agents.length) {
      return toolCall.delegated;
    }
    return {
      ...toolCall.delegated,
      agents: agents.length > 0 ? agents : seeded.agents,
      agentCount: agents.length > 0 ? agents.length : seeded.agents.length,
      totalTargets: agents.length > 0 ? agents.length : seeded.agents.length,
    };
  }, [toolCall.arguments, toolCall.delegated, toolCall.name]);
  const groupId = delegated?.delegatedRunId || toolCall.delegatedRunId;
  const projectedGroupState = delegated?.groupState || delegated?.stage;
  const groupState =
    toolCall.status === "error" &&
    (!projectedGroupState || ACTIVE_GROUP_STATES.has(projectedGroupState))
      ? "failed"
      : projectedGroupState;
  const active =
    ACTIVE_GROUP_STATES.has(groupState || "") || toolCall.status === "running";

  const counts = useMemo(() => {
    const agents = delegated?.agents ?? [];
    return {
      total: delegated?.agentCount ?? agents.length,
      running: agents.filter(
        (agent) => agent.taskState === "running" || agent.status === "running",
      ).length,
      complete: agents.filter(
        (agent) =>
          agent.taskState === "complete" || agent.status === "complete",
      ).length,
      partial: agents.filter(
        (agent) => (agent.taskState || agent.status) === "degraded",
      ).length,
      failed: agents.filter((agent) =>
        ["failed", "cancelled"].includes(agent.taskState || agent.status),
      ).length,
    };
  }, [delegated]);

  if (!delegated) return null;

  const summary = [
    counts.running ? `${counts.running} live` : undefined,
    counts.complete ? `${counts.complete} complete` : undefined,
    counts.partial ? `${counts.partial} partial` : undefined,
    counts.failed ? `${counts.failed} failed` : undefined,
  ]
    .filter(Boolean)
    .join(" · ");

  const openAgent = (taskId: string, name: string) => {
    if (!sessionId || !groupId) return;
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    router.push({
      pathname: "/agent/[sessionId]/[groupId]/[taskId]",
      params: { sessionId, groupId, taskId, name, fromParent: "1" },
    } as never);
  };

  return (
    <View style={styles.root}>
      <View style={styles.header}>
        <Users
          size={15}
          color={active ? t.info : t.mutedForeground}
          strokeWidth={1.9}
        />
        <Text selectable style={[styles.title, { color: t.foreground }]}>
          Agents
        </Text>
        <Text
          selectable
          numberOfLines={1}
          style={[styles.summary, { color: t.mutedForeground }]}
        >
          {summary || stateLabel(groupState)}
        </Text>
      </View>

      <View style={styles.agentList}>
        {delegated.agents.map((agent) => {
          const state = agent.taskState || agent.status;
          const canOpen = Boolean(
            sessionId &&
            groupId &&
            agent.taskId !== "main" &&
            !agent.taskId.startsWith("declared:"),
          );
          const color =
            state === "complete"
              ? t.success
              : ["failed", "cancelled"].includes(state)
                ? t.error
                : state === "degraded"
                  ? t.warning
                  : state === "running"
                    ? t.info
                    : t.mutedForeground;
          return (
            <Pressable
              key={agent.taskId}
              accessibilityRole={canOpen ? "button" : undefined}
              accessibilityLabel={`Open ${agent.name} conversation`}
              disabled={!canOpen}
              onPress={() => openAgent(agent.taskId, agent.name)}
              style={({ pressed }) => [
                styles.agentRow,
                pressed && canOpen && styles.pressed,
              ]}
            >
              <StatusIcon state={state} color={color} />
              <View style={styles.agentText}>
                <Text
                  selectable
                  numberOfLines={1}
                  style={[styles.agentName, { color: t.foreground }]}
                >
                  {agent.name}
                </Text>
                <Text
                  selectable
                  numberOfLines={1}
                  style={[styles.activity, { color: t.mutedForeground }]}
                >
                  {agent.currentAction || stateLabel(state)}
                </Text>
              </View>
              <View style={styles.stateCell}>
                <Text selectable style={[styles.agentState, { color }]}>
                  {stateLabel(state)}
                </Text>
                {canOpen ? (
                  <ChevronRight size={14} color={t.mutedForeground} />
                ) : null}
              </View>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
});

function StatusIcon({ state, color }: { state: string; color: string }) {
  if (state === "complete")
    return <Check size={13} color={color} strokeWidth={2.2} />;
  if (["failed", "cancelled"].includes(state))
    return <X size={13} color={color} strokeWidth={2.2} />;
  return (
    <Circle
      size={9}
      color={color}
      strokeWidth={state === "running" ? 3.4 : 1.8}
    />
  );
}

const styles = StyleSheet.create({
  root: { gap: 6, paddingVertical: 4 },
  header: {
    minHeight: 28,
    paddingHorizontal: 2,
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
  },
  title: { fontSize: 13, lineHeight: 17, fontWeight: "700" },
  summary: {
    flex: 1,
    minWidth: 0,
    textAlign: "right",
    fontSize: 10,
    lineHeight: 14,
    fontVariant: ["tabular-nums"],
  },
  agentList: { gap: 2 },
  agentRow: {
    minHeight: 48,
    paddingHorizontal: 2,
    paddingVertical: 6,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
  },
  agentText: { flex: 1, minWidth: 0, gap: 1 },
  agentName: { fontSize: 12, lineHeight: 16, fontWeight: "600" },
  activity: { fontSize: 10, lineHeight: 14 },
  stateCell: {
    flexDirection: "row",
    justifyContent: "flex-end",
    alignItems: "center",
    gap: 3,
  },
  agentState: {
    fontSize: 8,
    lineHeight: 11,
    fontWeight: "800",
    textTransform: "uppercase",
  },
  pressed: { opacity: 0.58 },
});
