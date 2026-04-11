import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoThreadSurface } from "./MakoThreadSurface";
import {
  formatProjectLabel,
  getRunGroup,
  getRunNextWakeAt,
} from "./utils";
import type {
  MakoChatContext,
  MakoCurrentState,
  MakoHomeState,
} from "./types";

interface MakoCurrentViewProps {
  state: MakoCurrentState;
  homeState: MakoHomeState;
  chat: MakoChatContext;
  threadJumpMessageId?: string | null;
  onThreadJumpHandled?: () => void;
  onSelectRun: (runId: string) => void;
  onOpenDetails: () => void;
  onOpenSchedule: () => void;
}

function formatWakeTime(value?: string | null) {
  if (!value) {
    return "No wake set";
  }

  return new Date(value).toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
}

export function MakoCurrentView({
  state,
  homeState,
  chat,
  threadJumpMessageId,
  onThreadJumpHandled,
  onSelectRun,
  onOpenDetails,
  onOpenSchedule,
}: MakoCurrentViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (state.isLoading && !state.current && homeState.isLoading && !homeState.home) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={t.userMessage} />
      </View>
    );
  }

  const runs = state.current?.runs ?? [];
  const approvals = state.current?.approvals ?? [];
  const status = state.current?.status;
  const waitingRuns = runs.filter((run) => getRunGroup(run) === "waiting");
  const activeRuns = runs.filter((run) => getRunGroup(run) === "active");
  const sleepingRuns = runs.filter((run) => getRunGroup(run) === "sleeping");
  const queuedRuns = runs.filter((run) => getRunGroup(run) === "queued");

  const focusApproval = approvals[0] ?? null;
  const focusRun = waitingRuns[0] ?? activeRuns[0] ?? null;
  const nextScheduledRun =
    [...queuedRuns, ...sleepingRuns]
      .sort((left, right) => {
        const leftValue = getRunNextWakeAt(left) ?? "9999";
        const rightValue = getRunNextWakeAt(right) ?? "9999";
        return leftValue.localeCompare(rightValue);
      })[0] ?? null;

  const home = homeState.home;
  const needsBootstrap =
    !homeState.isLoading &&
    !home?.soul &&
    !home?.identity &&
    !home?.heartbeat &&
    !home?.channels &&
    (home?.crew_count ?? 0) === 0;

  const topError = state.error ?? homeState.error;
  const stateBits = [
    status?.home_status ?? "idle",
    `${status?.running_count ?? 0} running`,
    `${approvals.length} attention`,
    `next wake ${formatWakeTime(status?.next_wake_at)}`,
  ];

  return (
    <View style={styles.container}>
      <View style={[styles.metaBlock, { borderBottomColor: t.border }]}>
        <Pressable onPress={onOpenDetails} style={styles.statusLine}>
          <Text style={[styles.statusText, { color: t.mutedForeground }]}>
            {stateBits.join(" • ")}
          </Text>
        </Pressable>

        {needsBootstrap ? (
          <View style={[styles.focusRow, { borderColor: t.border }]}>
            <View style={styles.focusCopy}>
              <Text style={[styles.focusTitle, { color: t.foreground }]}>
                Initialize Mako
              </Text>
              <Text style={[styles.focusDetail, { color: t.mutedForeground }]}>
                Create soul, identity, heartbeat, memory, channels, and crew.
              </Text>
            </View>
            <Pressable
              onPress={() => {
                void homeState.bootstrap();
              }}
              style={styles.focusAction}
              disabled={homeState.isBootstrapping}
            >
              <Text style={[styles.focusActionText, { color: t.userMessage }]}>
                {homeState.isBootstrapping ? "Initializing..." : "Initialize"}
              </Text>
            </Pressable>
          </View>
        ) : focusApproval ? (
          <View style={[styles.focusRow, { borderColor: t.border }]}>
            <View style={styles.focusCopy}>
              <Text style={[styles.focusTitle, { color: t.foreground }]}>
                Approval needed for {focusApproval.tool_name}
              </Text>
              <Text style={[styles.focusDetail, { color: t.mutedForeground }]}>
                {formatProjectLabel(focusApproval.project_dir)} is waiting on you.
              </Text>
            </View>
            <Pressable
              onPress={() => {
                onSelectRun(focusApproval.session_id);
              }}
              style={styles.focusAction}
            >
              <Text style={[styles.focusActionText, { color: t.userMessage }]}>
                Open
              </Text>
            </Pressable>
          </View>
        ) : focusRun ? (
          <View style={[styles.focusRow, { borderColor: t.border }]}>
            <View style={styles.focusCopy}>
              <Text style={[styles.focusTitle, { color: t.foreground }]}>
                {focusRun.title || "Untitled run"}
              </Text>
              <Text style={[styles.focusDetail, { color: t.mutedForeground }]}>
                {formatProjectLabel(focusRun.project_dir)} is active now.
              </Text>
            </View>
            <Pressable
              onPress={() => {
                onSelectRun(focusRun.session_id);
              }}
              style={styles.focusAction}
            >
              <Text style={[styles.focusActionText, { color: t.userMessage }]}>
                Open run
              </Text>
            </Pressable>
          </View>
        ) : nextScheduledRun ? (
          <View style={[styles.focusRow, { borderColor: t.border }]}>
            <View style={styles.focusCopy}>
              <Text style={[styles.focusTitle, { color: t.foreground }]}>
                Next wake
              </Text>
              <Text style={[styles.focusDetail, { color: t.mutedForeground }]}>
                {(nextScheduledRun.title || "Untitled run") +
                  " at " +
                  formatWakeTime(getRunNextWakeAt(nextScheduledRun))}
              </Text>
            </View>
            <Pressable onPress={onOpenSchedule} style={styles.focusAction}>
              <Text style={[styles.focusActionText, { color: t.userMessage }]}>
                Schedule
              </Text>
            </Pressable>
          </View>
        ) : null}

        {topError ? (
          <Text style={[styles.errorText, { color: t.error }]}>{topError}</Text>
        ) : null}
      </View>

      <View style={styles.threadWrap}>
        <MakoThreadSurface
          chat={chat}
          scrollToMessageId={threadJumpMessageId}
          onScrollTargetHandled={onThreadJumpHandled}
          emptyTitle="Talk to Mako"
          emptyBody="Use this thread to steer work, ask for updates, and open projects or runs when they matter."
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  metaBlock: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    paddingBottom: 10,
  },
  statusLine: {
    paddingTop: 4,
    paddingBottom: 8,
  },
  statusText: {
    fontSize: 12,
    lineHeight: 18,
  },
  focusRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
  },
  focusCopy: {
    flex: 1,
    minWidth: 0,
  },
  focusTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  focusDetail: {
    marginTop: 3,
    fontSize: 12,
    lineHeight: 17,
  },
  focusAction: {
    paddingVertical: 2,
  },
  focusActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  errorText: {
    marginTop: 8,
    fontSize: 12,
    lineHeight: 18,
  },
  threadWrap: {
    flex: 1,
    minHeight: 0,
  },
});
