import { useEffect, useState } from "react";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveAttentionView } from "./HiveAttentionView";
import { HiveChannelsView } from "./HiveChannelsView";
import { HiveCurrentView } from "./HiveCurrentView";
import { HiveCrewView } from "./HiveCrewView";
import { HiveGroupsView } from "./HiveGroupsView";
import { HiveLogbookView } from "./HiveLogbookView";
import { HiveMemoryView } from "./HiveMemoryView";
import { HiveRunView } from "./HiveRunView";
import { HiveRunsView } from "./HiveRunsView";
import { HiveScheduleView } from "./HiveScheduleView";
import { HiveStatusView } from "./HiveStatusView";
import { HiveTopBar } from "./HiveTopBar";
import { useHiveCurrent } from "./hooks/useHiveCurrent";
import { useHiveGroups } from "./hooks/useHiveGroups";
import { useHiveHome } from "./hooks/useHiveHome";
import { useHiveMemories } from "./hooks/useHiveMemories";
import { useHiveNavigation } from "./hooks/useHiveNavigation";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";
import type { HiveChatContext, HiveTopLevelView } from "./types";

interface HiveScreenProps {
  workspaceDirectory?: string | null;
  requestedTopLevel?: HiveTopLevelView;
  requestedThreadMessageId?: string;
  requestedReportId?: string;
  chat: HiveChatContext;
  onOpenRunById: (runId: string) => Promise<void>;
  onOpenWorkerDm: (sessionId: string) => void;
  onOpenProject?: (
    projectDir: string,
    targetBranch?: string | null,
  ) => Promise<void> | void;
  onDeleteRun: (runId: string) => void;
  workers: HiveWorkersState;
  onOpenMenu?: () => void;
  onTopLevelChange?: (view: HiveTopLevelView) => void;
}

export function HiveScreen({
  workspaceDirectory,
  requestedTopLevel,
  requestedThreadMessageId,
  requestedReportId,
  chat,
  onOpenRunById,
  onOpenWorkerDm,
  onOpenProject,
  onDeleteRun,
  workers,
  onOpenMenu,
  onTopLevelChange,
}: HiveScreenProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const current = useHiveCurrent(true);
  const home = useHiveHome(true);
  // Mobile mounts this surface only for management destinations. Starting at
  // the requested view prevents the transcript-owning Hive home from mounting
  // for one frame before the synchronization effect runs.
  const navigation = useHiveNavigation(requestedTopLevel);
  const groups = useHiveGroups(navigation.topLevel === "groups");
  const memories = useHiveMemories(
    navigation.topLevel === "memory",
    workspaceDirectory,
  );
  const [threadJumpMessageId, setThreadJumpMessageId] = useState<string | null>(
    null,
  );
  const [reportJumpId, setReportJumpId] = useState<string | null>(null);

  const selectedRun = navigation.selectedRunId
    ? current.current?.runs.find((run) =>
      run.session_id === navigation.selectedRunId
    ) ?? null
    : null;

  useEffect(() => {
    if (!requestedTopLevel) {
      return;
    }

    if (navigation.selectedRunId) {
      navigation.closeRun();
    }

    navigation.setTopLevel(requestedTopLevel);
  }, [
    navigation.closeRun,
    navigation.selectedRunId,
    navigation.setTopLevel,
    requestedTopLevel,
  ]);

  useEffect(() => {
    if (!requestedThreadMessageId && !requestedReportId) {
      return;
    }
    setThreadJumpMessageId(requestedThreadMessageId ?? null);
    setReportJumpId(requestedReportId ?? null);
    navigation.setTopLevel("hive");
  }, [navigation.setTopLevel, requestedReportId, requestedThreadMessageId]);

  useEffect(() => {
    onTopLevelChange?.(navigation.topLevel);
  }, [navigation.topLevel, onTopLevelChange]);

  const handleOpenRun = async (runId: string) => {
    navigation.openRun(runId);
    await onOpenRunById(runId);
  };

  const status = current.current?.status.home_status ?? "idle";
  const topLevelTitles: Record<HiveTopLevelView, string> = {
    hive: "Hive",
    attention: "Activity",
    schedule: "Calendar",
    logbook: "Logbook",
    runs: "Runs",
    details: "Details",
    crew: "Workers",
    groups: "Groups",
    channels: "Channels",
    memory: "Memory",
  };
  const title = topLevelTitles[navigation.topLevel] ?? "Hive";
  const subtitle = navigation.topLevel === "hive"
    ? "The hive is always alive."
    : undefined;

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: theme.colors.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {navigation.selectedRunId
        ? (
          <HiveRunView
            key={navigation.selectedRunId}
            runId={navigation.selectedRunId}
            summary={selectedRun}
            crewMembers={home.crew?.members ?? []}
            chat={chat}
            onBack={navigation.closeRun}
            onDeleteRun={onDeleteRun}
          />
        )
        : (
          <>
            <HiveTopBar
              title={title}
              subtitle={subtitle}
              status={status}
              titleStatus={navigation.topLevel === "hive" ? status : null}
              showStatusBadge={false}
              onOpenMenu={!isDesktop ? onOpenMenu : undefined}
            />

            {navigation.topLevel === "hive"
              ? (
                <HiveCurrentView
                  state={current}
                  homeState={home}
                  workers={workers}
                  chat={chat}
                  threadJumpMessageId={threadJumpMessageId}
                  reportJumpId={reportJumpId}
                  onThreadJumpHandled={() => {
                    setThreadJumpMessageId(null);
                  }}
                  onReportJumpHandled={() => {
                    setReportJumpId(null);
                  }}
                />
              )
              : null}

            {navigation.topLevel === "attention"
              ? (
                <HiveAttentionView
                  state={current}
                  chat={chat}
                  onSelectRun={(runId) => {
                    void handleOpenRun(runId);
                  }}
                  onOpenThread={(messageId) => {
                    setThreadJumpMessageId(messageId ?? null);
                    navigation.setTopLevel("hive");
                  }}
                />
              )
              : null}

            {navigation.topLevel === "schedule"
              ? (
                <HiveScheduleView
                  state={current}
                  onOpenProject={onOpenProject}
                  onSelectRun={(runId) => {
                    void handleOpenRun(runId);
                  }}
                />
              )
              : null}

            {navigation.topLevel === "runs"
              ? (
                <HiveRunsView
                  state={current}
                  onSelectRun={(runId) => {
                    void handleOpenRun(runId);
                  }}
                />
              )
              : null}

            {navigation.topLevel === "logbook"
              ? <HiveLogbookView workspaceDirectory={workspaceDirectory} />
              : null}
            {navigation.topLevel === "details"
              ? (
                <HiveStatusView
                  state={current}
                  activeToolCallId={chat.activeToolCallId}
                  onSelectRun={(runId) => {
                    void handleOpenRun(runId);
                  }}
                  onApproveTool={chat.onApproveTool}
                  onDenyTool={chat.onDenyTool}
                />
              )
              : null}
            {navigation.topLevel === "crew"
              ? (
                <HiveCrewView
                  state={home}
                  workers={workers}
                  models={chat.models}
                  onOpenWorkerDm={onOpenWorkerDm}
                />
              )
              : null}
            {navigation.topLevel === "groups"
              ? <HiveGroupsView state={groups} workers={workers} />
              : null}
            {navigation.topLevel === "channels"
              ? <HiveChannelsView state={home} />
              : null}
            {navigation.topLevel === "memory"
              ? (
                <HiveMemoryView
                  workspaceDirectory={workspaceDirectory}
                  state={memories}
                />
              )
              : null}
          </>
        )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
});
