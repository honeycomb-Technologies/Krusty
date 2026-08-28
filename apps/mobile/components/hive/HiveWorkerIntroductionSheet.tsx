import { type ReactNode, useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Animated, { FadeInUp, FadeOut } from "react-native-reanimated";
import type {
  HiveWorkerIntroductionFactKind,
  HiveWorkerIntroductionSelectedFact,
} from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import type { ActiveHiveWorkerIntroductionState } from "./hooks/useActiveHiveWorkerIntroduction";

interface HiveWorkerIntroductionSheetProps {
  state: ActiveHiveWorkerIntroductionState;
  bottom: number;
  onHeightChange: (height: number) => void;
}

const CATEGORY_LABELS: Record<HiveWorkerIntroductionFactKind, string> = {
  role: "Role",
  purpose: "Purpose",
  responsibility: "Responsibility",
  working_style: "Working style",
  boundary: "Boundary",
  tool_expectation: "Tool expectation",
  memory_expectation: "Memory expectation",
  cadence: "Cadence",
  user_preference: "Preference",
  user_correction: "Correction",
  relationship_context: "Context",
};

export function HiveWorkerIntroductionSheet({
  state,
  bottom,
  onHeightChange,
}: HiveWorkerIntroductionSheetProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const introduction = state.introduction;
  const proposal = introduction?.proposal ?? null;
  const projection = introduction?.review_projection;
  const proposalKey = proposal
    ? `${proposal.proposal_id}:${proposal.revision}`
    : "none";
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    setSelectedIds(new Set(proposal?.facts.map((fact) => fact.fact_id) ?? []));
  }, [proposalKey]);

  useEffect(
    () => () => {
      onHeightChange(0);
    },
    [onHeightChange],
  );

  const selectedFacts = useMemo<HiveWorkerIntroductionSelectedFact[]>(
    () =>
      proposal?.facts
        .filter((fact) => selectedIds.has(fact.fact_id))
        .map((fact) => ({
          fact_id: fact.fact_id,
          final_statement: fact.statement,
        })) ?? [],
    [proposal, selectedIds],
  );

  const shouldShowSheet = Boolean(
    state.worker &&
      (state.worker.status !== "active" ||
        (introduction &&
          introduction.status !== "confirmed" &&
          introduction.status !== "skipped" &&
          (introduction.status === "queued" ||
            introduction.status === "running" ||
            introduction.status === "awaiting_context" ||
            (introduction.status === "review_ready" && Boolean(proposal)) ||
            introduction.status === "failed" ||
            introduction.status === "needs_recovery" ||
            projection?.state === "pending" ||
            projection?.state === "claimed" ||
            projection?.state === "retrying" ||
            projection?.state === "needs_attention"))),
  );

  useEffect(() => {
    if (!shouldShowSheet) onHeightChange(0);
  }, [onHeightChange, shouldShowSheet]);

  if (!state.worker) return null;

  const shell = (content: ReactNode) => (
    <Animated.View
      entering={FadeInUp.duration(180)}
      exiting={FadeOut.duration(120)}
      onLayout={(event) =>
        onHeightChange(Math.ceil(event.nativeEvent.layout.height) + 8)}
      style={[
        styles.position,
        { bottom },
      ]}
    >
      <View
        style={[
          styles.sheet,
          {
            backgroundColor: t.background,
            borderColor: t.border,
          },
        ]}
      >
        {content}
        {state.error
          ? (
            <Text style={[styles.requestError, { color: t.error }]}>
              {state.error}
            </Text>
          )
          : null}
      </View>
    </Animated.View>
  );

  if (state.worker.status === "paused") {
    return shell(
      <View>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          Worker paused
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          This conversation is read-only until the Worker resumes.
        </Text>
        <View style={styles.inlineActions}>
          <InlineAction
            label="Resume Worker"
            disabled={state.isSaving}
            onPress={() => void state.resume().catch(() => undefined)}
          />
        </View>
      </View>,
    );
  }
  if (state.worker.status === "archived") {
    return shell(
      <StatusLine
        title="Worker archived"
        detail="Its conversation and history are retained, but new messages are disabled."
      />,
    );
  }
  if (!introduction) return null;
  if (
    introduction.status === "confirmed" || introduction.status === "skipped"
  ) {
    return null;
  }

  if (introduction.status === "queued" || introduction.status === "running") {
    return shell(
      <StatusLine
        title="Your Worker is introducing itself"
        detail="The first message comes from the Worker."
        busy
      />,
    );
  }

  if (
    introduction.status === "failed" ||
    introduction.status === "needs_recovery"
  ) {
    return shell(
      <View>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          Introduction paused
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          {introduction.last_error ||
            "The opening message could not be completed."}
        </Text>
        <View style={styles.inlineActions}>
          <InlineAction
            label="Retry"
            disabled={state.isSaving}
            onPress={() => void state.retry().catch(() => undefined)}
          />
          <InlineAction
            label="Skip setup"
            disabled={state.isSaving}
            onPress={() => void state.skip().catch(() => undefined)}
          />
        </View>
      </View>,
    );
  }

  const proposalCanConfirm = Boolean(
    introduction.status === "review_ready" &&
      proposal &&
      projection?.state === "review_ready" &&
      projection.is_current_through &&
      proposal.worker_id === state.worker.id &&
      proposal.session_id === state.detail?.dm_session_id,
  );
  const reviewIsStale = introduction.status === "review_ready" && proposal &&
    !proposalCanConfirm;

  if (reviewIsStale && proposal) {
    return shell(
      <View>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          Review needs attention
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          {projection?.last_error ||
            "The Worker or conversation changed after this summary was prepared."}
        </Text>
        <View style={styles.inlineActions}>
          <InlineAction
            label="Keep talking"
            disabled={state.isSaving}
            onPress={() => void state.keepTalking().catch(() => undefined)}
          />
        </View>
      </View>,
    );
  }

  if (proposalCanConfirm && proposal) {
    return shell(
      <View>
        <Text style={[styles.title, { color: t.foreground }]}>
          What I understand
        </Text>
        <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
          Choose what becomes part of this Worker’s setup.
        </Text>
        <ScrollView style={styles.factList} nestedScrollEnabled>
          {proposal.facts.map((fact) => {
            const selected = selectedIds.has(fact.fact_id);
            return (
              <Pressable
                key={fact.fact_id}
                accessibilityRole="checkbox"
                accessibilityState={{ checked: selected }}
                aria-checked={selected}
                onPress={() => {
                  setSelectedIds((current) => {
                    const next = new Set(current);
                    if (next.has(fact.fact_id)) next.delete(fact.fact_id);
                    else next.add(fact.fact_id);
                    return next;
                  });
                }}
                style={[
                  styles.fact,
                  {
                    borderTopColor: t.border,
                    backgroundColor: selected
                      ? `${t.userMessage}0D`
                      : "transparent",
                  },
                ]}
              >
                <View
                  style={[
                    styles.check,
                    {
                      borderColor: selected ? t.userMessage : t.border,
                      backgroundColor: selected ? t.userMessage : "transparent",
                    },
                  ]}
                >
                  <Text style={[styles.checkText, { color: t.onAccent }]}>
                    {selected ? "✓" : ""}
                  </Text>
                </View>
                <View style={styles.factCopy}>
                  <Text style={[styles.category, { color: t.userMessage }]}>
                    {CATEGORY_LABELS[fact.kind]}
                  </Text>
                  <Text style={[styles.statement, { color: t.foreground }]}>
                    {fact.statement}
                  </Text>
                  <Text style={[styles.evidence, { color: t.mutedForeground }]}>
                    “{fact.evidence_excerpt}”
                  </Text>
                </View>
              </Pressable>
            );
          })}
        </ScrollView>
        <View style={[styles.actions, { borderTopColor: t.border }]}>
          <Pressable
            disabled={state.isSaving}
            onPress={() => void state.keepTalking().catch(() => undefined)}
            style={styles.quietAction}
          >
            <Text
              style={[styles.quietActionText, { color: t.mutedForeground }]}
            >
              Keep talking
            </Text>
          </Pressable>
          <Pressable
            disabled={state.isSaving || selectedFacts.length === 0}
            onPress={() =>
              void state.confirm(selectedFacts).catch(() => undefined)}
            style={[
              styles.primaryAction,
              {
                backgroundColor: t.userMessage,
                opacity: state.isSaving || selectedFacts.length === 0
                  ? 0.45
                  : 1,
              },
            ]}
          >
            <Text style={[styles.primaryActionText, { color: t.onAccent }]}>
              Confirm selected
            </Text>
          </Pressable>
        </View>
      </View>,
    );
  }

  if (
    projection?.state === "pending" ||
    projection?.state === "claimed" ||
    projection?.state === "retrying"
  ) {
    const retrying = projection.state === "retrying";
    return shell(
      <StatusLine
        title={retrying ? "Reviewing again" : "Reviewing what you discussed"}
        detail={retrying
          ? `Attempt ${
            Math.max(1, projection.attempt_count + 1)
          } is in progress.`
          : "This is a private, tool-free review of the conversation."}
        busy
      />,
    );
  }

  if (projection?.state === "needs_attention") {
    return shell(
      <View>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          Review needs attention
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          {projection.last_error ||
            "This review could not finish automatically."}
        </Text>
        <Text style={[styles.continueHint, { color: t.userMessage }]}>
          Keep talking in the composer below to create a new review boundary.
        </Text>
        <View style={styles.inlineActions}>
          <InlineAction
            label="Skip setup"
            disabled={state.isSaving}
            onPress={() => void state.skip().catch(() => undefined)}
          />
        </View>
      </View>,
    );
  }

  if (introduction.status === "awaiting_context") {
    return shell(
      <View>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          Shape this Worker
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          Keep talking in the composer while you define the role and purpose,
          working style, boundaries, tools, memory, and cadence—or skip setup
          and continue now.
        </Text>
        <View style={styles.inlineActions}>
          <InlineAction
            label="Skip setup"
            disabled={state.isSaving}
            onPress={() => void state.skip().catch(() => undefined)}
          />
        </View>
      </View>,
    );
  }

  return null;
}

function StatusLine({
  title,
  detail,
  busy,
}: {
  title: string;
  detail: string;
  busy?: boolean;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  return (
    <View style={styles.statusRow}>
      {busy ? <ActivityIndicator color={t.userMessage} size="small" /> : null}
      <View style={styles.statusCopy}>
        <Text style={[styles.statusTitle, { color: t.foreground }]}>
          {title}
        </Text>
        <Text style={[styles.statusDetail, { color: t.mutedForeground }]}>
          {detail}
        </Text>
      </View>
    </View>
  );
}

function InlineAction({
  label,
  disabled,
  onPress,
}: {
  label: string;
  disabled: boolean;
  onPress: () => void;
}) {
  const { theme } = useThemeContext();
  return (
    <Pressable
      disabled={disabled}
      onPress={onPress}
      style={{ opacity: disabled ? 0.45 : 1 }}
    >
      <Text
        style={[styles.inlineActionText, { color: theme.colors.userMessage }]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  position: {
    position: "absolute",
    left: 16,
    right: 16,
    zIndex: 4,
  },
  sheet: {
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  title: {
    fontSize: 15,
    lineHeight: 20,
    fontWeight: "600",
  },
  subtitle: {
    marginTop: 2,
    fontSize: 12,
    lineHeight: 17,
  },
  factList: {
    maxHeight: 276,
    marginTop: 8,
  },
  fact: {
    flexDirection: "row",
    gap: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 4,
    paddingVertical: 9,
  },
  check: {
    width: 19,
    height: 19,
    marginTop: 1,
    borderWidth: 1,
    borderRadius: 6,
    alignItems: "center",
    justifyContent: "center",
  },
  checkText: {
    fontSize: 12,
    lineHeight: 15,
    fontWeight: "700",
  },
  factCopy: {
    flex: 1,
    minWidth: 0,
  },
  category: {
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "700",
    textTransform: "uppercase",
    letterSpacing: 0.55,
  },
  statement: {
    marginTop: 2,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
  evidence: {
    marginTop: 3,
    fontSize: 11,
    lineHeight: 16,
  },
  actions: {
    marginTop: 8,
    paddingTop: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: 12,
  },
  quietAction: {
    paddingHorizontal: 4,
    paddingVertical: 8,
  },
  quietActionText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "600",
  },
  primaryAction: {
    minHeight: 36,
    borderRadius: 10,
    paddingHorizontal: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  primaryActionText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "700",
  },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  statusCopy: {
    flex: 1,
    minWidth: 0,
  },
  statusTitle: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "600",
  },
  statusDetail: {
    marginTop: 2,
    fontSize: 12,
    lineHeight: 17,
  },
  inlineActions: {
    marginTop: 8,
    flexDirection: "row",
    gap: 14,
  },
  inlineActionText: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "600",
  },
  requestError: {
    marginTop: 8,
    fontSize: 11,
    lineHeight: 16,
  },
  continueHint: {
    marginTop: 7,
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "600",
  },
});
