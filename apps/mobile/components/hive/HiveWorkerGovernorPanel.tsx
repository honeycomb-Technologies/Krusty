import { Pressable, StyleSheet, Text, View } from "react-native";
import type {
  HiveWorker,
  HiveWorkerGovernorGateReason,
  HiveWorkerGovernorLaneDecision,
} from "@mitsuro/api";

import { useThemeContext } from "../../hooks/useTheme";
import { useHiveWorkerGovernor } from "./hooks/useHiveWorkerGovernor";

function count(value: number): string {
  return value.toLocaleString();
}

function decimal(value: string): string {
  return value.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

function minuteOfDay(value: number): string {
  const hour = Math.floor(value / 60).toString().padStart(2, "0");
  const minute = (value % 60).toString().padStart(2, "0");
  return `${hour}:${minute}`;
}

function duration(seconds: number): string {
  if (seconds % 3_600 === 0) return `${seconds / 3_600}h`;
  if (seconds % 60 === 0) return `${seconds / 60}m`;
  return `${seconds}s`;
}

function timestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function gateReason(reason: HiveWorkerGovernorGateReason): string {
  switch (reason) {
    case "policy_unavailable":
      return "policy unavailable";
    case "unresolved_provider_call":
      return "provider call needs reconciliation";
    case "daily_call_cap_reached":
      return "daily call cap reached";
    case "daily_token_cap_reached":
      return "daily token cap reached";
    case "quiet_hours":
      return "quiet hours";
    case "idle_backoff":
      return "idle backoff";
  }
}

function decisionLabel(lane: HiveWorkerGovernorLaneDecision): string {
  const { decision } = lane;
  if (decision.reasons.length === 0) {
    return "allowed for a minimum-size call check";
  }
  return `${decision.disposition} · ${
    decision.reasons.map(gateReason).join(
      ", ",
    )
  }`;
}

interface HiveWorkerGovernorPanelProps {
  worker: HiveWorker;
  sessionId: string | null;
  enabled: boolean;
  compact?: boolean;
  poll?: boolean;
}

/** Limits, spend truth, and the daemon-owned unresolved-call recovery action. */
export function HiveWorkerGovernorPanel({
  worker,
  sessionId,
  enabled,
  compact = false,
  poll = true,
}: HiveWorkerGovernorPanelProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const state = useHiveWorkerGovernor({ worker, sessionId, enabled, poll });
  const projection = state.projection;

  if (!enabled || !sessionId) return null;
  if (!projection) {
    return (
      <View
        style={[
          styles.panel,
          compact && styles.compactPanel,
          { borderColor: t.border, backgroundColor: t.card },
        ]}
      >
        <Text style={[styles.title, { color: t.mutedForeground }]}>Limits</Text>
        <Text style={[styles.line, { color: t.mutedForeground }]}>
          {state.error ??
            (state.isLoading ? "Loading usage…" : "Usage unavailable")}
        </Text>
        {state.error
          ? (
            <Pressable accessibilityRole="button" onPress={state.refresh}>
              <Text style={[styles.retry, { color: t.userMessage }]}>
                Retry
              </Text>
            </Pressable>
          )
          : null}
      </View>
    );
  }

  const policy = projection.policy;
  const daily = projection.daily;
  const costs = projection.estimated_daily_cost.by_currency.map((item) =>
    `${item.currency} ${decimal(item.estimated_cost_microunits)} µ`
  );
  if (projection.estimated_daily_cost.unpriced_call_count > 0) {
    costs.push(
      `${count(projection.estimated_daily_cost.unpriced_call_count)} unpriced`,
    );
  }
  const quiet = policy.quiet_start_minute == null ||
      policy.quiet_end_minute == null
    ? "off"
    : `${minuteOfDay(policy.quiet_start_minute)}–${
      minuteOfDay(
        policy.quiet_end_minute,
      )
    }`;
  const nextWake = projection.autonomous_dm.decision.next_eligible_at
    ? timestamp(projection.autonomous_dm.decision.next_eligible_at)
    : projection.autonomous_dm.decision.reasons.includes(
        "unresolved_provider_call",
      )
    ? "requires reconciliation"
    : projection.response_loss_recovery_required
    ? "requires response recovery"
    : "now";
  const idle = projection.autonomous_dm.decision.idle;
  const hasUnresolvedStart = projection.unresolved_started_count > 0;
  const hasResponseLoss = projection.response_loss_recovery_required;
  const hasCombinedRecovery = hasResponseLoss && hasUnresolvedStart;
  const hasRecoveryBoundary = hasUnresolvedStart || hasResponseLoss;

  return (
    <View
      accessibilityLabel={`${worker.display_name} Worker limits`}
      style={[
        styles.panel,
        compact && styles.compactPanel,
        { borderColor: t.border, backgroundColor: t.card },
      ]}
    >
      <View style={styles.titleRow}>
        <Text style={[styles.title, { color: t.foreground }]}>
          Limits today
        </Text>
        <Text style={[styles.revision, { color: t.mutedForeground }]}>
          Policy r{policy.revision}
          {state.isLoading ? " · refreshing" : ""}
        </Text>
      </View>
      <Text style={[styles.line, { color: t.mutedForeground }]}>
        Calls {count(daily.calls_used)} / {count(daily.calls_limit)} · Tokens
        {" "}
        {count(daily.tokens_used_or_reserved)} / {count(daily.tokens_limit)}
      </Text>
      <Text style={[styles.line, { color: t.mutedForeground }]}>
        Estimated cost {costs.length > 0 ? costs.join(" · ") : "none recorded"}
        {" · "}µ is one-millionth of a currency unit
      </Text>
      <Text style={[styles.line, { color: t.mutedForeground }]}>
        Auto wake {decisionLabel(projection.autonomous_dm)} · Next {nextWake}
      </Text>
      <Text style={[styles.line, { color: t.mutedForeground }]}>
        Quiet {quiet} {policy.timezone} · Idle{" "}
        {duration(policy.idle_base_secs)}–
        {duration(policy.idle_max_secs)} · streak {idle.idle_streak}
        {idle.not_before ? ` until ${timestamp(idle.not_before)}` : ""}
      </Text>
      {hasRecoveryBoundary
        ? (
          <View style={styles.recoverySection}>
            <Text style={[styles.warning, { color: t.error }]}>
              {hasCombinedRecovery
                ? `Provider reply was not committed · ${
                  count(projection.unresolved_started_count)
                } older unresolved provider${
                  projection.unresolved_started_count === 1
                    ? " start"
                    : " starts"
                }`
                : hasResponseLoss
                ? "Provider completed, but its reply was not committed"
                : `${
                  count(projection.unresolved_started_count)
                } unresolved provider${
                  projection.unresolved_started_count === 1
                    ? " start"
                    : " starts"
                }`}
            </Text>
            <Text style={[styles.recoveryCopy, { color: t.mutedForeground }]}>
              {hasCombinedRecovery
                ? "Acknowledge the missing reply without replay, then prepare one short-lived recovery for the older uncertain call. Only the next new direct message can use its unresolved-call bypass; call and token caps, quiet hours, idle backoff, Goal, group, review, and background runs stay enforced."
                : hasResponseLoss
                ? "Acknowledge the missing reply to retire this exact direct-message boundary and continue with the next staged message. This creates no bypass grant and never replays the completed provider call."
                : "Prepare one short-lived call for the next new direct message. It bypasses only unresolved-call reconciliation; call and token caps, quiet hours, idle backoff, Goal, group, review, and background runs stay enforced. A used recovery acknowledges the older uncertain start for future gating without erasing its immutable accounting record."}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={hasResponseLoss
                ? `Acknowledge missing reply for ${worker.display_name}`
                : `Prepare one-call recovery for ${worker.display_name}`}
              disabled={state.isGrantingRecovery ||
                Boolean(state.recoveryGrant)}
              onPress={() => {
                void state.grantRecovery();
              }}
              style={({ pressed }) => [
                styles.recoveryButton,
                {
                  borderColor: t.border,
                  opacity: state.isGrantingRecovery || state.recoveryGrant
                    ? 0.55
                    : pressed
                    ? 0.7
                    : 1,
                },
              ]}
            >
              <Text style={[styles.retry, { color: t.userMessage }]}>
                {state.isGrantingRecovery
                  ? hasResponseLoss ? "Acknowledging…" : "Preparing…"
                  : state.recoveryGrant
                  ? state.recoveryGrant.status === "response_loss_acknowledged"
                    ? "Response loss acknowledged"
                    : state.recoveryGrant.status ===
                        "response_loss_acknowledged_with_grant"
                    ? "Reply loss acknowledged · Recovery ready"
                    : "Recovery ready"
                  : hasResponseLoss
                  ? "Acknowledge missing reply"
                  : "Prepare next direct message"}
              </Text>
            </Pressable>
            {state.recoveryGrant
              ? (
                <Text
                  style={[styles.confirmation, { color: t.mutedForeground }]}
                >
                  {state.recoveryGrant.status === "response_loss_acknowledged"
                    ? "Missing reply acknowledged. The completed provider call was not replayed and no bypass was created."
                    : state.recoveryGrant.status ===
                        "response_loss_acknowledged_with_grant"
                    ? `Missing reply acknowledged without replay. An older uncertain call still requires the recovery prepared until ${
                      timestamp(state.recoveryGrant.expires_at)
                    }; only the next new direct message can use it.`
                    : `${
                      state.recoveryGrant.status === "already_available"
                        ? "Recovery was already ready"
                        : "Recovery ready"
                    } until ${
                      timestamp(state.recoveryGrant.expires_at)
                    }. Only the next new direct message can use its one unresolved-call bypass.`}
                </Text>
              )
              : null}
            {state.recoveryError
              ? (
                <Text style={[styles.warning, { color: t.error }]}>
                  {state.recoveryError}
                </Text>
              )
              : null}
          </View>
        )
        : null}
      {!compact
        ? (
          <>
            <Text style={[styles.line, { color: t.mutedForeground }]}>
              Your messages {decisionLabel(projection.foreground_dm)} · Reset
              {" "}
              {timestamp(daily.resets_at)}
            </Text>
            <Text style={[styles.line, { color: t.mutedForeground }]}>
              DST gap {policy.quiet_gap_policy.replace("_", " ")} · fold{" "}
              {policy.quiet_fold_policy} · Unresolved starts{" "}
              {count(projection.unresolved_started_count)}
              {projection.response_loss_recovery_required
                ? " · Missing reply acknowledgment required"
                : ""}
            </Text>
            <Text style={[styles.readOnly, { color: t.mutedForeground }]}>
              Policy is read only. The daemon remains the authority for limits
              and recovery.
            </Text>
          </>
        )
        : null}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 11,
    paddingVertical: 10,
    gap: 4,
  },
  compactPanel: {
    paddingVertical: 8,
  },
  titleRow: {
    flexDirection: "row",
    alignItems: "baseline",
    justifyContent: "space-between",
    gap: 8,
  },
  title: {
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 0.35,
    textTransform: "uppercase",
  },
  revision: {
    fontSize: 10,
  },
  line: {
    fontSize: 11,
    lineHeight: 15,
  },
  warning: {
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "600",
  },
  recoverySection: {
    gap: 4,
    marginTop: 3,
  },
  recoveryCopy: {
    fontSize: 10,
    lineHeight: 14,
  },
  recoveryButton: {
    alignSelf: "flex-start",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 9,
    paddingVertical: 6,
  },
  confirmation: {
    fontSize: 10,
    lineHeight: 14,
  },
  retry: {
    fontSize: 11,
    fontWeight: "600",
  },
  readOnly: {
    fontSize: 10,
    lineHeight: 14,
    marginTop: 2,
  },
});
