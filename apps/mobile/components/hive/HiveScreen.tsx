import { useEffect, useState } from "react";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { HiveAttentionView } from "./HiveAttentionView";
import { HiveChannelsView } from "./HiveChannelsView";
import { HiveCurrentView } from "./HiveCurrentView";
import { HiveCrewView } from "./HiveCrewView";
import { HiveLogbookView } from "./HiveLogbookView";
import { HiveRunView } from "./HiveRunView";
import { HiveRunsView } from "./HiveRunsView";
import { HiveScheduleView } from "./HiveScheduleView";
import { HiveStatusView } from "./HiveStatusView";
import { HiveTopBar } from "./HiveTopBar";
import { useHiveCurrent } from "./hooks/useHiveCurrent";
import { useHiveHome } from "./hooks/useHiveHome";
import { useHiveNavigation } from "./hooks/useHiveNavigation";
import { useHiveWorkers } from "./hooks/useHiveWorkers";
import type { HiveChatContext, HiveTopLevelView } from "./types";

interface HiveScreenProps {
  workspaceDirectory?: string | null;
  requestedTopLevel?: HiveTopLevelView;
  requestedThreadMessageId?: string;
  requestedReportId?: string;
  chat: HiveChatContext;
  onOpenRunById: (runId: string) => Promise<void>;
  onOpenProject?: (projectDir: string, targetBranch?: string | null) => Promise<void> | void;
  onDeleteRun: (runId: string) => void;
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
  onOpenProject,
  onDeleteRun,
  onOpenMenu,
  onTopLevelChange,
}: HiveScreenProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const current = useHiveCurrent(true);
  const home = useHiveHome(true);
  const navigation = useHiveNavigation();
  // Workers are fetched only while the roster is visible; opening a DM works
  // from cached rows without keeping a background poll alive.
  const workers = useHiveWorkers(navigation.topLevel === "crew");
  const [threadJumpMessageId, setThreadJumpMessageId] = useState<string | null>(null);
  const [reportJumpId, setReportJumpId] = useState<string | null>(null);

  const selectedRun =
    navigation.selectedRunId
      ? current.current?.runs.find((run) => run.session_id === navigation.selectedRunId) ?? null
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
    attention: "Attention",
    schedule: "Schedule",
    logbook: "Logbook",
    runs: "Runs",
    details: "Details",
    crew: "Workers",
    channels: "Channels",
  };
  const title = topLevelTitles[navigation.topLevel] ?? "Hive";
  const subtitle = navigation.topLevel === "hive" ? "The hive is always alive." : undefined;

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: theme.colors.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {navigation.selectedRunId ? (
        <HiveRunView
          key={navigation.selectedRunId}
          runId={navigation.selectedRunId}
          summary={selectedRun}
          crewMembers={home.crew?.members ?? []}
          chat={chat}
          onBack={navigation.closeRun}
          onDeleteRun={onDeleteRun}
        />
      ) : (
        <>
          <HiveTopBar
            title={title}
            subtitle={subtitle}
            status={status}
            titleStatus={navigation.topLevel === "hive" ? status : null}
            showStatusBadge={false}
            onOpenMenu={!isDesktop ? onOpenMenu : undefined}
          />

          {navigation.topLevel === "hive" ? (
            <HiveCurrentView
              state={current}
              homeState={home}
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
          ) : null}

          {navigation.topLevel === "attention" ? (
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
          ) : null}

          {navigation.topLevel === "schedule" ? (
            <HiveScheduleView
              state={current}
              onOpenProject={onOpenProject}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
            />
          ) : null}

          {navigation.topLevel === "runs" ? (
            <HiveRunsView
              state={current}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
            />
          ) : null}

          {navigation.topLevel === "logbook" ? (
            <HiveLogbookView workspaceDirectory={workspaceDirectory} />
          ) : null}
          {navigation.topLevel === "details" ? (
            <HiveStatusView
              state={current}
              activeToolCallId={chat.activeToolCallId}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onApproveTool={chat.onApproveTool}
              onDenyTool={chat.onDenyTool}
            />
          ) : null}
          {navigation.topLevel === "crew" ? (
            <HiveCrewView
              state={home}
              workers={workers}
              models={chat.models}
              onOpenWorkerDm={(sessionId) => {
                // Load the Worker's DM into the hive session store, then land
                // on the thread surface that renders it.
                void onOpenRunById(sessionId);
                navigation.setTopLevel("hive");
              }}
            />
          ) : null}
          {navigation.topLevel === "channels" ? <HiveChannelsView state={home} /> : null}
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
