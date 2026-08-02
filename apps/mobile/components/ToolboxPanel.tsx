import { useCallback, useEffect } from "react";
import {
  Platform,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import {
  Archive,
  CalendarClock,
  Cable,
  FileCode2,
  Globe2,
  TerminalSquare,
  X,
  type LucideIcon,
} from "lucide-react-native";
import type { SessionType } from "@mitsuro/api";

import { useThemeContext } from "../hooks/useTheme";
import { useBreakpoint } from "../hooks/useBreakpoint";
import * as Haptics from "../platform/haptics";
import { AppBottomSheet } from "./sheets/AppBottomSheet";
import { ToolboxTerminal } from "./toolbox/ToolboxTerminal";
import { ToolboxBrowser } from "./toolbox/ToolboxBrowser";
import { ToolboxChanges } from "./toolbox/ToolboxChanges";
import { ToolboxConnections } from "./toolbox/ToolboxConnections";
import { ReportsContent } from "./ReportsViewer";
import { HiveScheduleView } from "./hive/HiveScheduleView";
import { useHiveCurrent } from "./hive/hooks/useHiveCurrent";

interface ToolTab {
  label: string;
  icon: LucideIcon;
}

const TOOL_TABS: Record<SessionType, ToolTab[]> = {
  chat: [
    { label: "Library", icon: Archive },
    { label: "Connections", icon: Cable },
  ],
  code: [
    { label: "Browser", icon: Globe2 },
    { label: "Terminal", icon: TerminalSquare },
    { label: "Changes", icon: FileCode2 },
  ],
  hive: [
    { label: "Schedule", icon: CalendarClock },
  ],
};

interface ToolboxPanelProps {
  visible: boolean;
  onClose: () => void;
  activeTab: number;
  onTabChange: (tab: number) => void;
  sessionType: SessionType;
  projectDirectory?: string | null;
  onOpenSettings?: () => void;
  onOpenHiveRun?: (sessionId: string) => void;
  onOpenProject?: (projectDir: string, targetBranch?: string | null) => void;
  /**
   * `dock` is the wide-web rail. `overlay` is the shared mobile bottom drawer.
   */
  variant?: "dock" | "overlay";
}

function HiveToolboxBody({
  visible,
  onOpenHiveRun,
  onOpenProject,
}: {
  visible: boolean;
  onOpenHiveRun?: (sessionId: string) => void;
  onOpenProject?: (projectDir: string, targetBranch?: string | null) => void;
}) {
  const current = useHiveCurrent(visible);
  const openRun = (sessionId: string) => onOpenHiveRun?.(sessionId);

  return (
    <View style={styles.body}>
      {visible ? (
        <View style={styles.tabContent}>
          <HiveScheduleView
            state={current}
            onSelectRun={openRun}
            onOpenProject={onOpenProject}
          />
        </View>
      ) : null}
    </View>
  );
}

export function ToolboxPanel({
  visible,
  onClose,
  activeTab,
  onTabChange,
  sessionType,
  projectDirectory,
  onOpenSettings,
  onOpenHiveRun,
  onOpenProject,
  variant,
}: ToolboxPanelProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const mode = variant ?? (isDesktop ? "dock" : "overlay");
  const isDock = mode === "dock";
  const tabs = TOOL_TABS[sessionType];
  const showTabRail = sessionType !== "hive";

  useEffect(() => {
    if (activeTab >= tabs.length) {
      onTabChange(0);
    }
  }, [activeTab, onTabChange, tabs.length]);

  const handleClose = useCallback(() => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onClose();
  }, [onClose]);

  const handleTabChange = useCallback(
    (index: number) => {
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      onTabChange(index);
    },
    [onTabChange],
  );

  if (isDock && !visible) {
    return null;
  }

  const tabRail = showTabRail ? (
    <View
      accessibilityRole="tablist"
      style={[
        styles.tabRail,
        {
          backgroundColor: t.glass.background,
          borderColor: t.glass.border,
        },
      ]}
    >
      {tabs.map((tab, index) => {
        const Icon = tab.icon;
        const active = index === activeTab;
        return (
          <Pressable
            key={tab.label}
            accessibilityRole="tab"
            accessibilityLabel={tab.label}
            accessibilityState={{ selected: active }}
            onPress={() => handleTabChange(index)}
            style={[
              styles.tabButton,
              active && { backgroundColor: t.glass.backgroundElevated },
            ]}
          >
            <Icon
              size={18}
              color={active ? t.foreground : t.mutedForeground}
              strokeWidth={active ? 2.1 : 1.8}
            />
          </Pressable>
        );
      })}
    </View>
  ) : null;

  const header = (
    <View style={[styles.header, { borderBottomColor: t.border }]}>
      {tabRail}
      <Pressable
        onPress={handleClose}
        accessibilityRole="button"
        accessibilityLabel="Close toolbox"
        style={styles.closeBtn}
      >
        <X size={18} color={t.mutedForeground} strokeWidth={1.8} />
      </Pressable>
    </View>
  );

  const drawerDock = (
    <View style={[styles.drawerDock, { borderTopColor: t.border }]}>
      {tabRail}
    </View>
  );

  let body;
  if (sessionType === "chat") {
    body = (
      <View style={styles.body}>
        <View style={[styles.tabContent, activeTab !== 0 && styles.hidden]}>
          <ReportsContent visible={visible && activeTab === 0} />
        </View>
        <View style={[styles.tabContent, activeTab !== 1 && styles.hidden]}>
          <ToolboxConnections
            visible={visible && activeTab === 1}
            onOpenSettings={onOpenSettings}
          />
        </View>
      </View>
    );
  } else if (sessionType === "code") {
    // Keep browser/terminal mounted while the toolbox is open so tab switches
    // only hide them. Destroying WebViews on every switch looked like a reopen.
    body = (
      <View style={styles.body}>
        <View style={[styles.tabContent, activeTab !== 0 && styles.hidden]}>
          <ToolboxBrowser visible={visible && activeTab === 0} />
        </View>
        <View style={[styles.tabContent, activeTab !== 1 && styles.hidden]}>
          <ToolboxTerminal visible={visible && activeTab === 1} />
        </View>
        <View style={[styles.tabContent, activeTab !== 2 && styles.hidden]}>
          <ToolboxChanges
            visible={visible && activeTab === 2}
            projectDirectory={projectDirectory}
          />
        </View>
      </View>
    );
  } else {
    body = (
      <HiveToolboxBody
        visible={visible}
        onOpenHiveRun={onOpenHiveRun}
        onOpenProject={onOpenProject}
      />
    );
  }

  const content = (
    <View style={styles.surface}>
      {isDock ? header : null}
      {body}
    </View>
  );

  if (isDock) {
    return (
      <View
        style={[
          styles.dockPanel,
          { borderLeftColor: t.border, backgroundColor: t.background },
        ]}
      >
        {content}
      </View>
    );
  }

  return (
    <AppBottomSheet
      visible={visible}
      onClose={handleClose}
      footer={showTabRail ? drawerDock : undefined}
      accessibilityLabel={`${sessionType} toolbox`}
      testID="mobile-toolbox-sheet"
      // Code toolbox hosts browser/terminal: mount once, then keep warm.
      retainContent={sessionType === 'code'}
    >
      {content}
    </AppBottomSheet>
  );
}

const styles = StyleSheet.create({
  dockPanel: {
    width: 360,
    flexGrow: 0,
    flexShrink: 0,
    flexBasis: 360,
    alignSelf: "stretch",
    flexDirection: "column",
    overflow: "hidden",
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  surface: {
    flex: 1,
    minHeight: 0,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 10,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 10,
  },
  drawerDock: {
    minHeight: 58,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  tabRail: {
    flexDirection: "row",
    alignItems: "center",
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 2,
    gap: 2,
  },
  tabButton: {
    width: 44,
    minHeight: 36,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  closeBtn: {
    position: "absolute",
    right: 10,
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  body: {
    flex: 1,
    minHeight: 0,
  },
  tabContent: {
    ...StyleSheet.absoluteFillObject,
  },
  hidden:
    Platform.OS === "web"
      ? ({ display: "none" } as const)
      : { opacity: 0, pointerEvents: "none" as const },
});
