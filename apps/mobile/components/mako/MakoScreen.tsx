import { useEffect } from "react";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoChannelsView } from "./MakoChannelsView";
import { MakoCurrentView } from "./MakoCurrentView";
import { MakoCrewView } from "./MakoCrewView";
import { MakoLogbookView } from "./MakoLogbookView";
import { MakoRunView } from "./MakoRunView";
import { MakoRunsView } from "./MakoRunsView";
import { MakoScheduleView } from "./MakoScheduleView";
import { MakoStatusView } from "./MakoStatusView";
import { MakoTopBar } from "./MakoTopBar";
import { MakoTopNav } from "./MakoTopNav";
import { useMakoCurrent } from "./hooks/useMakoCurrent";
import { useMakoHome } from "./hooks/useMakoHome";
import { useMakoNavigation } from "./hooks/useMakoNavigation";
import type { MakoChatContext } from "./types";

interface MakoScreenProps {
  workspaceDirectory?: string | null;
  activeRunId?: string | null;
  chat: MakoChatContext;
  onOpenRunById: (runId: string) => Promise<void>;
  onDeleteRun: (runId: string) => void;
  onOpenMenu?: () => void;
}

export function MakoScreen({
  workspaceDirectory,
  activeRunId,
  chat,
  onOpenRunById,
  onDeleteRun,
  onOpenMenu,
}: MakoScreenProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const current = useMakoCurrent(true);
  const home = useMakoHome(true);
  const navigation = useMakoNavigation(activeRunId);

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

  const handleOpenRun = async (runId: string) => {
    navigation.openRun(runId);
    await onOpenRunById(runId);
  };

  const status = current.current?.status.home_status ?? "idle";
  const navActive: "mako" | "schedule" | "logbook" =
    navigation.topLevel === "schedule"
      ? "schedule"
      : navigation.topLevel === "logbook"
        ? "logbook"
        : "mako";

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
            title="Mako"
            subtitle="Always Swimming."
            status={status}
            titleStatus={status}
            showStatusBadge={false}
            onOpenMenu={!isDesktop ? onOpenMenu : undefined}
          />

          <MakoTopNav
            items={[
              { id: "mako", label: "Mako" },
              { id: "schedule", label: "Schedule" },
              { id: "logbook", label: "Logbook" },
            ]}
            active={navActive}
            onSelect={navigation.setTopLevel}
          />

          {navigation.topLevel === "mako" ? (
            <MakoCurrentView
              state={current}
              homeState={home}
              chat={chat}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onOpenRuns={() => {
                navigation.setTopLevel("runs");
              }}
              onOpenCrew={() => {
                navigation.setTopLevel("crew");
              }}
              onOpenChannels={() => {
                navigation.setTopLevel("channels");
              }}
              onOpenSchedule={() => {
                navigation.setTopLevel("schedule");
              }}
              onOpenDetails={() => {
                navigation.setTopLevel("details");
              }}
            />
          ) : null}

          {navigation.topLevel === "schedule" ? (
            <MakoScheduleView
              state={current}
              workspaceDirectory={workspaceDirectory}
              model={chat.model ?? null}
              crewMembers={home.crew?.members ?? []}
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
