import { useEffect, useState } from "react";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoAttentionView } from "./MakoAttentionView";
import { MakoChannelsView } from "./MakoChannelsView";
import { MakoCurrentView } from "./MakoCurrentView";
import { MakoCrewView } from "./MakoCrewView";
import { MakoLogbookView } from "./MakoLogbookView";
import { MakoRunView } from "./MakoRunView";
import { MakoRunsView } from "./MakoRunsView";
import { MakoScheduleView } from "./MakoScheduleView";
import { MakoStatusView } from "./MakoStatusView";
import { MakoTopBar } from "./MakoTopBar";
import { useMakoCurrent } from "./hooks/useMakoCurrent";
import { useMakoHome } from "./hooks/useMakoHome";
import { useMakoNavigation } from "./hooks/useMakoNavigation";
import type { MakoChatContext, MakoTopLevelView } from "./types";

interface MakoScreenProps {
  workspaceDirectory?: string | null;
  activeRunId?: string | null;
  requestedTopLevel?: MakoTopLevelView;
  chat: MakoChatContext;
  onOpenRunById: (runId: string) => Promise<void>;
  onOpenProject?: (projectDir: string) => Promise<void> | void;
  onDeleteRun: (runId: string) => void;
  onOpenMenu?: () => void;
  onTopLevelChange?: (view: MakoTopLevelView) => void;
}

export function MakoScreen({
  workspaceDirectory,
  activeRunId,
  requestedTopLevel,
  chat,
  onOpenRunById,
  onOpenProject,
  onDeleteRun,
  onOpenMenu,
  onTopLevelChange,
}: MakoScreenProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const current = useMakoCurrent(true);
  const home = useMakoHome(true);
  const navigation = useMakoNavigation(activeRunId);
  const [threadJumpMessageId, setThreadJumpMessageId] = useState<string | null>(null);

  const selectedRun =
    navigation.selectedRunId
      ? current.current?.runs.find((run) => run.session_id === navigation.selectedRunId) ?? null
      : null;

  useEffect(() => {
    if (!navigation.selectedRunId) {
      void current.refresh();
      void home.refresh();
    }
  }, [current.refresh, home.refresh, navigation.selectedRunId]);

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
    onTopLevelChange?.(navigation.topLevel);
  }, [navigation.topLevel, onTopLevelChange]);

  const handleOpenRun = async (runId: string) => {
    navigation.openRun(runId);
    await onOpenRunById(runId);
  };

  const status = current.current?.status.home_status ?? "idle";
  const topLevelTitles: Record<MakoTopLevelView, string> = {
    mako: "Mako",
    attention: "Attention",
    schedule: "Schedule",
    logbook: "Logbook",
    runs: "Runs",
    details: "Details",
    crew: "Crew",
    channels: "Channels",
  };
  const title = topLevelTitles[navigation.topLevel] ?? "Mako";
  const subtitle = navigation.topLevel === "mako" ? "Always Swimming." : undefined;

  return (
    <SafeAreaView
      style={[styles.container, { backgroundColor: theme.colors.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {navigation.selectedRunId ? (
        <MakoRunView
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
          <MakoTopBar
            title={title}
            subtitle={subtitle}
            status={status}
            titleStatus={navigation.topLevel === "mako" ? status : null}
            showStatusBadge={false}
            onOpenMenu={!isDesktop ? onOpenMenu : undefined}
          />

          {navigation.topLevel === "mako" ? (
            <MakoCurrentView
              state={current}
              homeState={home}
              chat={chat}
              threadJumpMessageId={threadJumpMessageId}
              onThreadJumpHandled={() => {
                setThreadJumpMessageId(null);
              }}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onOpenSchedule={() => {
                navigation.setTopLevel("schedule");
              }}
              onOpenDetails={() => {
                navigation.setTopLevel("details");
              }}
            />
          ) : null}

          {navigation.topLevel === "attention" ? (
            <MakoAttentionView
              state={current}
              chat={chat}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onOpenThread={(messageId) => {
                setThreadJumpMessageId(messageId ?? null);
                navigation.setTopLevel("mako");
              }}
            />
          ) : null}

          {navigation.topLevel === "schedule" ? (
            <MakoScheduleView
              state={current}
              onOpenProject={onOpenProject}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
            />
          ) : null}

          {navigation.topLevel === "runs" ? (
            <MakoRunsView
              state={current}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
            />
          ) : null}

          {navigation.topLevel === "logbook" ? (
            <MakoLogbookView workspaceDirectory={workspaceDirectory} />
          ) : null}
          {navigation.topLevel === "details" ? (
            <MakoStatusView
              state={current}
              activeToolCallId={chat.activeToolCallId}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onApproveTool={chat.onApproveTool}
              onDenyTool={chat.onDenyTool}
            />
          ) : null}
          {navigation.topLevel === "crew" ? <MakoCrewView state={home} /> : null}
          {navigation.topLevel === "channels" ? <MakoChannelsView state={home} /> : null}
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
