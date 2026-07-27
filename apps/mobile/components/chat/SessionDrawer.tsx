import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
} from "react-native";
import Animated, {
  Easing,
  interpolate,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";
import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Folder,
  FolderOpen,
  FolderPlus,
  Settings,
  SquarePlus,
  Wifi,
  WifiOff,
} from "lucide-react-native";
import type {
  MakoSessionSummary,
  SessionResponse,
  SessionType,
} from "@krusty/api";

import { useThemeContext } from "../../hooks/useTheme";
import { SessionListSkeleton } from "../ui/Skeleton";
import { useConnection } from "../../hooks/useConnection";
import * as Haptics from "../../platform/haptics";
import { AppBottomSheet } from "../sheets/AppBottomSheet";
import {
  chronologicalSessions,
  codeDirectoryToAutoExpand,
  codeProjectThreadGroups,
} from "../navigation/threadSections";

interface SessionDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onSelectMakoSession: (sessionId: string) => void;
  onNewSession: (type: "chat" | "code") => void;
  onNewMakoSession: () => void;
  onNewSessionWithDir: (path: string) => void;
  onDeleteSession: (id: string) => void;
  onOpenSettings?: () => void;
  activeMode: SessionType;
}

interface DirEntry {
  name: string;
  path: string;
}

interface DirCache {
  current: string;
  parent: string | null;
  directories: DirEntry[];
}

function dirDisplayName(path: string): string {
  if (path === "Neutral") {
    return "General";
  }
  const parts = path.split("/").filter(Boolean);
  return parts.slice(-2).join("/");
}

function formatTime(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function modeTitle(mode: SessionType): string {
  if (mode === "chat") return "Chat threads";
  if (mode === "code") return "Code threads";
  return "Mako threads";
}

export function SessionDrawer({
  isOpen,
  onClose,
  sessions,
  activeSessionId,
  onSelectSession,
  onSelectMakoSession,
  onNewSession,
  onNewMakoSession,
  onNewSessionWithDir,
  onDeleteSession,
  onOpenSettings,
  activeMode,
}: SessionDrawerProps) {
  const { theme } = useThemeContext();
  const { client, status } = useConnection();
  const { height: windowHeight } = useWindowDimensions();
  const t = theme.colors;
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const lastAutoExpandedCodeSessionRef = useRef<string | null>(null);
  const [makoSessions, setMakoSessions] = useState<MakoSessionSummary[]>([]);
  const [makoLoading, setMakoLoading] = useState(false);

  const pickerProgress = useSharedValue(0);
  const [pickerVisible, setPickerVisible] = useState(false);
  const [pickerPath, setPickerPath] = useState("");
  const [pickerParent, setPickerParent] = useState<string | null>(null);
  const [pickerDirs, setPickerDirs] = useState<DirEntry[]>([]);
  const MAX_DIR_CACHE = 40;
  const dirCache = useRef<Map<string, DirCache>>(new Map());
  const setDirCache = (key: string, value: DirCache) => {
    dirCache.current.delete(key);
    setDirCache(key, value);
    while (dirCache.current.size > MAX_DIR_CACHE) {
      const oldest = dirCache.current.keys().next().value;
      if (!oldest) break;
      dirCache.current.delete(oldest);
    }
  };
  const [pickerReady, setPickerReady] = useState(false);
  const pickerHeight = Math.max(300, Math.round(windowHeight * 0.58));

  const chatSessions = useMemo(
    () => chronologicalSessions(sessions, "chat"),
    [sessions],
  );
  const codeGroups = useMemo(
    () => codeProjectThreadGroups(sessions),
    [sessions],
  );

  useEffect(() => {
    if (!isOpen || activeMode !== "mako" || !client) {
      return;
    }
    let active = true;
    setMakoLoading(true);
    void client
      .listMakoSessions()
      .then((nextSessions) => {
        if (active) {
          setMakoSessions(nextSessions);
        }
      })
      .catch(() => {
        if (active) {
          setMakoSessions([]);
        }
      })
      .finally(() => {
        if (active) {
          setMakoLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [activeMode, client, isOpen]);

  useEffect(() => {
    if (activeMode !== "code" || !activeSessionId) {
      return;
    }
    const directory = codeDirectoryToAutoExpand(
      sessions,
      activeSessionId,
      lastAutoExpandedCodeSessionRef.current,
    );
    if (!directory) {
      return;
    }
    lastAutoExpandedCodeSessionRef.current = activeSessionId;
    setExpandedDirs((current) => {
      if (current.has(directory)) {
        return current;
      }
      const next = new Set(current);
      next.add(directory);
      return next;
    });
  }, [activeMode, activeSessionId, sessions]);

  const loadPickerRoot = useCallback(async () => {
    if (!client || pickerReady) {
      return;
    }
    try {
      const result = await client.browseDirectories();
      const entry: DirCache = {
        current: result.current,
        parent: result.parent,
        directories: result.directories,
      };
      setDirCache("", entry);
      setDirCache(result.current, entry);
      setPickerPath(result.current);
      setPickerParent(result.parent);
      setPickerDirs(result.directories);
      setPickerReady(true);
    } catch {
      // The current drawer remains usable if directory browsing is unavailable.
    }
  }, [client, pickerReady]);

  useEffect(() => {
    if (isOpen && activeMode === "code") {
      void loadPickerRoot();
    }
  }, [activeMode, isOpen, loadPickerRoot]);

  useEffect(() => {
    if (!isOpen) {
      pickerProgress.value = withTiming(0, { duration: 150 });
      const timer = setTimeout(() => setPickerVisible(false), 160);
      return () => clearTimeout(timer);
    }
  }, [isOpen, pickerProgress]);

  const navigatePicker = useCallback(
    async (path: string) => {
      if (!client) {
        return;
      }
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      const cached = dirCache.current.get(path);
      if (cached) {
        setPickerPath(cached.current);
        setPickerParent(cached.parent);
        setPickerDirs(cached.directories);
        return;
      }
      try {
        const result = await client.browseDirectories(path || undefined);
        const entry: DirCache = {
          current: result.current,
          parent: result.parent,
          directories: result.directories,
        };
        setDirCache(path, entry);
        setDirCache(result.current, entry);
        setPickerPath(result.current);
        setPickerParent(result.parent);
        setPickerDirs(result.directories);
      } catch {
        // Keep the last successfully loaded directory.
      }
    },
    [client],
  );

  const showPicker = useCallback(() => {
    setPickerVisible(true);
    pickerProgress.value = withSpring(1, {
      damping: 20,
      stiffness: 250,
      mass: 0.8,
    });
    void loadPickerRoot();
  }, [loadPickerRoot, pickerProgress]);

  const hidePicker = useCallback(() => {
    pickerProgress.value = withTiming(0, {
      duration: 200,
      easing: Easing.out(Easing.cubic),
    });
    setTimeout(() => setPickerVisible(false), 210);
  }, [pickerProgress]);

  const pickerStyle = useAnimatedStyle(() => ({
    opacity: pickerProgress.value,
    transform: [
      {
        translateY: interpolate(
          pickerProgress.value,
          [0, 1],
          [pickerHeight, 0],
        ),
      },
    ],
  }));

  const renderSession = (session: SessionResponse) => {
    const active = session.id === activeSessionId;
    return (
      <Pressable
        key={session.id}
        accessibilityRole="button"
        accessibilityState={{ selected: active }}
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onSelectSession(session);
        }}
        onLongPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
          onDeleteSession(session.id);
        }}
        style={[
          styles.sessionItem,
          active && { backgroundColor: `${t.userMessage}12` },
        ]}
      >
        <Text
          numberOfLines={2}
          style={[
            styles.sessionTitle,
            { color: active ? t.userMessage : t.foreground },
          ]}
        >
          {session.title || "Untitled"}
        </Text>
        <View style={styles.sessionMeta}>
          <Text style={[styles.sessionTime, { color: t.mutedForeground }]}>
            {formatTime(session.updated_at)}
          </Text>
          {session.model ? (
            <Text
              numberOfLines={1}
              style={[styles.sessionModel, { color: t.mutedForeground }]}
            >
              {session.model}
            </Text>
          ) : null}
        </View>
      </Pressable>
    );
  };

  const content = (() => {
    if (activeMode === "chat") {
      return chatSessions.length > 0 ? (
        chatSessions.map(renderSession)
      ) : (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
          No Chat threads yet
        </Text>
      );
    }

    if (activeMode === "code") {
      return codeGroups.length > 0 ? (
        codeGroups.map((group) => {
          const expanded = expandedDirs.has(group.directory);
          return (
            <View key={group.directory}>
              <Pressable
                accessibilityRole="button"
                accessibilityState={{ expanded }}
                onPress={() => {
                  void Haptics.impactAsync(
                    Haptics.ImpactFeedbackStyle.Light,
                  );
                  setExpandedDirs((current) => {
                    const next = new Set(current);
                    if (next.has(group.directory)) {
                      next.delete(group.directory);
                    } else {
                      next.add(group.directory);
                    }
                    return next;
                  });
                }}
                style={styles.dirHeader}
              >
                {expanded ? (
                  <FolderOpen
                    size={18}
                    color={t.thinking}
                    strokeWidth={1.7}
                  />
                ) : (
                  <Folder
                    size={18}
                    color={t.mutedForeground}
                    strokeWidth={1.6}
                  />
                )}
                <View style={styles.dirCopy}>
                  <Text
                    numberOfLines={1}
                    style={[
                      styles.dirName,
                      { color: expanded ? t.foreground : t.mutedForeground },
                    ]}
                  >
                    {dirDisplayName(group.directory)}
                  </Text>
                  <Text
                    numberOfLines={1}
                    style={[styles.dirPath, { color: t.mutedForeground }]}
                  >
                    {group.directory === "Neutral"
                      ? "No project selected"
                      : group.directory}
                  </Text>
                </View>
                <Text style={[styles.dirCount, { color: t.mutedForeground }]}>
                  {group.sessions.length}
                </Text>
                {expanded ? (
                  <ChevronDown size={16} color={t.mutedForeground} />
                ) : (
                  <ChevronRight size={16} color={t.mutedForeground} />
                )}
              </Pressable>
              {expanded ? (
                <View style={styles.dirSessions}>
                  {group.sessions.map(renderSession)}
                </View>
              ) : null}
            </View>
          );
        })
      ) : (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
          No Code threads yet
        </Text>
      );
    }

    if (makoLoading && makoSessions.length === 0) {
      return <SessionListSkeleton count={4} />;
    }

    return makoSessions.length > 0 ? (
      makoSessions.map((session) => {
        const active = session.session_id === activeSessionId;
        const runtime = session.runtime;
        const runtimeLabel = runtime?.status ?? session.agent_state;
        const crew = runtime?.crew_slug || "Mako default";
        return (
          <Pressable
            key={session.session_id}
            accessibilityRole="button"
            accessibilityState={{ selected: active }}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onSelectMakoSession(session.session_id);
            }}
            style={[
              styles.sessionItem,
              active && { backgroundColor: `${t.userMessage}12` },
            ]}
          >
            <View style={styles.makoTitleRow}>
              <Text
                numberOfLines={2}
                style={[
                  styles.sessionTitle,
                  { color: active ? t.userMessage : t.foreground },
                ]}
              >
                {session.title || "Untitled Mako"}
              </Text>
              <View
                style={[
                  styles.statusDot,
                  {
                    backgroundColor:
                      runtimeLabel === "running" ? t.success : t.mutedForeground,
                  },
                ]}
              />
            </View>
            <View style={styles.sessionMeta}>
              <Text style={[styles.sessionTime, { color: t.mutedForeground }]}>
                {formatTime(session.updated_at)}
              </Text>
              <Text
                numberOfLines={1}
                style={[styles.sessionModel, { color: t.mutedForeground }]}
              >
                {crew} · {runtimeLabel}
              </Text>
            </View>
          </Pressable>
        );
      })
    ) : (
      <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
        No Mako threads yet
      </Text>
    );
  })();

  const footer = (
    <View style={[styles.bottomBar, { borderTopColor: t.border }]}>
      {onOpenSettings ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Open settings"
          onPress={onOpenSettings}
          style={styles.iconButton}
        >
          <Settings size={22} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      ) : null}

      <View style={styles.statusIcon}>
        {status === "connected" ? (
          <Wifi size={16} color={t.success} strokeWidth={2} />
        ) : (
          <WifiOff
            size={16}
            color={status === "connecting" ? t.warning : t.error}
            strokeWidth={2}
          />
        )}
      </View>

      <View style={styles.spacer} />

      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Close threads"
        onPress={onClose}
        style={styles.iconButton}
      >
        <ChevronDown size={22} color={t.mutedForeground} strokeWidth={1.8} />
      </Pressable>

      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`New ${activeMode} thread`}
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
          if (activeMode === "mako") {
            onNewMakoSession();
          } else {
            onNewSession(activeMode);
          }
        }}
        style={styles.iconButton}
      >
        <SquarePlus size={22} color={t.mutedForeground} strokeWidth={1.8} />
      </Pressable>

      {activeMode === "code" ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="New Code thread in directory"
          onPress={showPicker}
          style={styles.iconButton}
        >
          <FolderPlus
            size={22}
            color={t.mutedForeground}
            strokeWidth={1.8}
          />
        </Pressable>
      ) : null}
    </View>
  );

  return (
    <AppBottomSheet
      visible={isOpen}
      onClose={onClose}
      footer={footer}
      accessibilityLabel={modeTitle(activeMode)}
      testID="mobile-threads-sheet"
    >
      <View style={styles.content}>
        <View style={styles.heading}>
          <Text style={[styles.headingTitle, { color: t.foreground }]}>
            {modeTitle(activeMode)}
          </Text>
          <Text style={[styles.headingDetail, { color: t.mutedForeground }]}>
            {activeMode === "code"
              ? "Projects ordered by recent work"
              : "Most recent first"}
          </Text>
        </View>

        <ScrollView
          style={styles.list}
          contentContainerStyle={styles.listContent}
          showsVerticalScrollIndicator={false}
        >
          {content}
        </ScrollView>

        {pickerVisible ? (
          <Animated.View
            style={[
              styles.picker,
              {
                height: pickerHeight,
                borderTopColor: t.border,
                backgroundColor: t.background,
              },
              pickerStyle,
            ]}
          >
            <View style={styles.pickerHeader}>
              <View style={styles.pickerHeadingCopy}>
                <Text style={[styles.pickerTitle, { color: t.foreground }]}>
                  Select directory
                </Text>
                <Text
                  numberOfLines={1}
                  style={[styles.pickerPath, { color: t.mutedForeground }]}
                >
                  {pickerPath}
                </Text>
              </View>
              <Pressable onPress={hidePicker} style={styles.iconButton}>
                <ChevronDown
                  size={20}
                  color={t.mutedForeground}
                  strokeWidth={1.8}
                />
              </Pressable>
            </View>

            <ScrollView
              style={styles.pickerList}
              showsVerticalScrollIndicator={false}
            >
              {pickerParent ? (
                <Pressable
                  onPress={() => void navigatePicker(pickerParent)}
                  style={styles.pickerItem}
                >
                  <ChevronLeft size={16} color={t.mutedForeground} />
                  <Text
                    style={[
                      styles.pickerItemText,
                      { color: t.mutedForeground },
                    ]}
                  >
                    Up
                  </Text>
                </Pressable>
              ) : null}
              {pickerDirs.map((directory) => (
                <Pressable
                  key={directory.path}
                  onPress={() => void navigatePicker(directory.path)}
                  style={styles.pickerItem}
                >
                  <Folder
                    size={16}
                    color={t.mutedForeground}
                    strokeWidth={1.6}
                  />
                  <Text
                    numberOfLines={1}
                    style={[
                      styles.pickerItemText,
                      { color: t.foreground },
                    ]}
                  >
                    {directory.name}
                  </Text>
                </Pressable>
              ))}
            </ScrollView>

            <Pressable
              accessibilityRole="button"
              onPress={() => {
                void Haptics.notificationAsync(
                  Haptics.NotificationFeedbackType.Success,
                );
                hidePicker();
                onNewSessionWithDir(pickerPath);
              }}
              style={[styles.selectButton, { borderColor: t.border }]}
            >
              <Check size={16} color={t.foreground} strokeWidth={2.4} />
              <Text style={[styles.selectButtonText, { color: t.foreground }]}>
                Use this directory
              </Text>
            </Pressable>
          </Animated.View>
        ) : null}
      </View>
    </AppBottomSheet>
  );
}

const styles = StyleSheet.create({
  content: {
    flex: 1,
    minHeight: 0,
  },
  heading: {
    paddingHorizontal: 18,
    paddingTop: 4,
    paddingBottom: 12,
  },
  headingTitle: {
    fontSize: 20,
    fontWeight: "700",
    letterSpacing: -0.25,
  },
  headingDetail: {
    marginTop: 3,
    fontSize: 12,
    fontWeight: "500",
  },
  list: {
    flex: 1,
  },
  listContent: {
    paddingHorizontal: 12,
    paddingBottom: 20,
  },
  emptyText: {
    fontSize: 14,
    textAlign: "center",
    marginTop: 44,
  },
  sessionItem: {
    paddingHorizontal: 12,
    paddingVertical: 11,
    borderRadius: 12,
    marginBottom: 3,
  },
  sessionTitle: {
    flex: 1,
    minWidth: 0,
    fontSize: 14,
    fontWeight: "600",
    lineHeight: 19,
  },
  sessionMeta: {
    marginTop: 5,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  sessionTime: {
    fontSize: 11,
  },
  sessionModel: {
    flex: 1,
    fontSize: 11,
  },
  makoTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  statusDot: {
    width: 7,
    height: 7,
    borderRadius: 4,
  },
  dirHeader: {
    minHeight: 58,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
    paddingHorizontal: 8,
    paddingVertical: 8,
  },
  dirCopy: {
    flex: 1,
    minWidth: 0,
  },
  dirName: {
    fontSize: 14,
    fontWeight: "700",
  },
  dirPath: {
    marginTop: 2,
    fontSize: 10,
  },
  dirCount: {
    fontSize: 11,
    fontWeight: "600",
  },
  dirSessions: {
    paddingLeft: 14,
    marginBottom: 5,
  },
  bottomBar: {
    minHeight: 58,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  statusIcon: {
    width: 34,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  spacer: {
    flex: 1,
  },
  picker: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    overflow: "hidden",
    paddingBottom: 10,
  },
  pickerHeader: {
    minHeight: 64,
    paddingLeft: 18,
    paddingRight: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  pickerHeadingCopy: {
    flex: 1,
    minWidth: 0,
  },
  pickerTitle: {
    fontSize: 16,
    fontWeight: "700",
  },
  pickerPath: {
    marginTop: 3,
    fontSize: 11,
  },
  pickerList: {
    flex: 1,
    paddingHorizontal: 12,
  },
  pickerItem: {
    minHeight: 44,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
  },
  pickerItemText: {
    flex: 1,
    fontSize: 13,
  },
  selectButton: {
    minHeight: 46,
    marginHorizontal: 16,
    marginTop: 8,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  selectButtonText: {
    fontSize: 13,
    fontWeight: "700",
  },
});
