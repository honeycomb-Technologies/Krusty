import { useEffect } from "react";
import { StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { MakoChatView } from "./MakoChatView";
import { MakoCurrentView } from "./MakoCurrentView";
import { MakoReportsView } from "./MakoReportsView";
import { MakoRunView } from "./MakoRunView";
import { MakoRunsView } from "./MakoRunsView";
import { MakoScheduleView } from "./MakoScheduleView";
import { MakoStatusView } from "./MakoStatusView";
import { MakoTopBar } from "./MakoTopBar";
import { MakoTopNav } from "./MakoTopNav";
import { useMakoCurrent } from "./hooks/useMakoCurrent";
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
  const navigation = useMakoNavigation(activeRunId);

  const selectedRun =
    navigation.selectedRunId
      ? current.current?.runs.find((run) => run.session_id === navigation.selectedRunId) ?? null
      : null;

  useEffect(() => {
    if (!navigation.selectedRunId) {
      void current.refresh();
    }
  }, [current.refresh, navigation.selectedRunId]);

  const handleOpenRun = async (runId: string) => {
    navigation.openRun(runId);
    await onOpenRunById(runId);
  };

  const status = current.current?.status.home_status ?? "idle";

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
              { id: "current", label: "Mako" },
              { id: "schedule", label: "Schedule" },
              { id: "reports", label: "Logbook" },
            ]}
            active={navigation.topLevel}
            onSelect={navigation.setTopLevel}
          />

          {navigation.topLevel === "current" ? (
            <MakoCurrentView
              state={current}
              workspaceDirectory={workspaceDirectory}
              model={chat.model ?? null}
              activeToolCallId={chat.activeToolCallId}
              chat={chat}
              onSelectRun={(runId) => {
                void handleOpenRun(runId);
              }}
              onOpenChat={() => {
                navigation.setTopLevel("chat");
              }}
              onOpenRuns={() => {
                navigation.setTopLevel("runs");
              }}
              onOpenDetails={() => {
                navigation.setTopLevel("status");
              }}
              onCourseSet={handleOpenRun}
              onApproveTool={chat.onApproveTool}
            />
          ) : null}

          {navigation.topLevel === "chat" ? <MakoChatView chat={chat} /> : null}

          {navigation.topLevel === "schedule" ? (
            <MakoScheduleView
              state={current}
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

          {navigation.topLevel === "reports" ? (
            <MakoReportsView workspaceDirectory={workspaceDirectory} />
          ) : null}
          {navigation.topLevel === "status" ? (
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
