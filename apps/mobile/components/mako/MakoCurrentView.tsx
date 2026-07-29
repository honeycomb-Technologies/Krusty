import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoThreadSurface } from "./MakoThreadSurface";
import { InlineReportCard } from "../reports/InlineReportCard";
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
  reportJumpId?: string | null;
  onThreadJumpHandled?: () => void;
  onReportJumpHandled?: () => void;
}

export function MakoCurrentView({
  state,
  homeState,
  chat,
  threadJumpMessageId,
  reportJumpId,
  onThreadJumpHandled,
  onReportJumpHandled,
}: MakoCurrentViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const showBootstrapLoading =
    state.isLoading && !state.current && homeState.isLoading && !homeState.home;

  const status = state.current?.status;
  const pendingApprovalCount = state.current?.approvals.length ?? 0;
  const waitingRun = state.current?.runs.find((run) => {
    const diagnosticKind = run.diagnostic?.kind;
    return (
      diagnosticKind === "awaiting_input" ||
      diagnosticKind === "awaiting_approval" ||
      run.blocked_tasks > 0
    );
  });
  const blockedPrompt =
    pendingApprovalCount > 0
      ? {
          title: "Hive needs your approval",
          detail:
            pendingApprovalCount === 1
              ? "Review the pending request and reply in this thread to continue."
              : `${pendingApprovalCount} requests are waiting. Reply in this thread to continue.`,
        }
      : waitingRun
        ? {
            title: "Hive needs your input",
            detail:
              waitingRun.diagnostic?.detail ||
              waitingRun.diagnostic?.summary ||
              "Reply in this thread with the missing direction. Hive will wait here until you do.",
          }
        : null;

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
  ];

  return (
    <View style={styles.container}>
      <View style={[styles.metaBlock, { borderBottomColor: t.border }]}>
        <View style={styles.statusLine}>
          <Text style={[styles.statusText, { color: t.mutedForeground }]}>
            {showBootstrapLoading ? "Connecting to Hive…" : stateBits.join(" • ")}
          </Text>
          {showBootstrapLoading || state.isRefreshing || homeState.isRefreshing ? (
            <ActivityIndicator color={t.userMessage} size="small" />
          ) : null}
        </View>

        {needsBootstrap ? (
          <View style={[styles.focusRow, { borderColor: t.border }]}>
            <View style={styles.focusCopy}>
              <Text style={[styles.focusTitle, { color: t.foreground }]}>
                Initialize Hive
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
        ) : null}

        {blockedPrompt ? (
          <View
            style={[
              styles.blockedPrompt,
              {
                borderColor: `${t.warning}55`,
                backgroundColor: `${t.warning}10`,
              },
            ]}
          >
            <Text style={[styles.focusTitle, { color: t.foreground }]}>
              {blockedPrompt.title}
            </Text>
            <Text style={[styles.focusDetail, { color: t.mutedForeground }]}>
              {blockedPrompt.detail}
            </Text>
          </View>
        ) : null}

        {topError ? (
          <Text style={[styles.errorText, { color: t.error }]}>{topError}</Text>
        ) : null}
      </View>

      <View style={styles.threadWrap}>
        {reportJumpId ? (
          <View style={styles.reportJump}>
            <InlineReportCard
              reportId={reportJumpId}
              defaultExpanded
              onDismiss={onReportJumpHandled}
            />
          </View>
        ) : null}
        <MakoThreadSurface
          chat={chat}
          scrollToMessageId={threadJumpMessageId}
          onScrollTargetHandled={onThreadJumpHandled}
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
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
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
  blockedPrompt: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 10,
    marginBottom: 4,
  },
  threadWrap: {
    flex: 1,
    minHeight: 0,
  },
  reportJump: {
    marginHorizontal: 16,
    marginTop: 10,
  },
});
