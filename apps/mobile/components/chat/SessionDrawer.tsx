import { colors } from '@mitsuro/ui';
import {
  createRef,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  FlatList,
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
import ReanimatedSwipeable, {
  type SwipeableMethods,
} from "react-native-gesture-handler/ReanimatedSwipeable";
import { Pressable as GesturePressable } from "react-native-gesture-handler";
import {
  Archive,
  ArchiveRestore,
  Bot,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Folder,
  FolderOpen,
  FolderPlus,
  FolderTree,
  List,
  Plus,
  Pin,
  PinOff,
  Rows3,
  Settings,
  SquarePlus,
  Trash2,
  Users,
  Wifi,
  WifiOff,
} from "lucide-react-native";
import type {
  GitChangedFile,
  HiveGroup,
  HiveSessionSummary,
  HiveWorker,
  SessionResponse,
  SessionType,
} from "@mitsuro/api";

import { useThemeContext } from "../../hooks/useTheme";
import { SessionListSkeleton } from "../ui/Skeleton";
import { useConnection } from "../../hooks/useConnection";
import * as Haptics from "../../platform/haptics";
import { HiveGroupRoomView } from "../hive/HiveGroupRoomView";
import {
  workerAvatarColor,
  workerFallbackColor,
  workerInitials,
  workerMetaLine,
} from "../hive/workerAppearance";
import { AppBottomSheet } from "../sheets/AppBottomSheet";
import {
  applySessionListOverrides,
  archivedSessions as archivedSessionsForType,
  chronologicalSessions,
  chronologicalThreadDayGroups,
  type SessionListOverride,
  type ChronologicalThreadDayGroup,
  type CodeProjectThreadGroup,
  type CodeThreadView,
  codeDirectoryToAutoExpand,
  codeProjectThreadGroups,
  formatThreadMetric,
  sessionModelLabel,
  sessionProjectDirectory,
  sessionProviderKey,
  sessionProviderLabel,
  sessionStateLabel,
  type ThreadDensity,
} from "../navigation/threadSections";

const THREAD_DENSITY_STORAGE_KEY = "mitsuro.thread-list-density.v1";

interface ProjectActivity {
  branch: string | null;
  files: number;
  additions: number;
  deletions: number;
}

type CodeListItem =
  | { kind: "project"; group: CodeProjectThreadGroup }
  | { kind: "day"; group: ChronologicalThreadDayGroup }
  | { kind: "session"; session: SessionResponse; directory: string }
  | { kind: "active-empty"; label: string }
  | ArchiveListItem;

type ArchiveListItem =
  | { kind: "archive-toggle" }
  | { kind: "archived-session"; session: SessionResponse }
  | { kind: "archive-loading" }
  | { kind: "archive-empty" };

type ChatListItem =
  | { kind: "session"; session: SessionResponse }
  | { kind: "active-empty"; label: string }
  | ArchiveListItem;

type HiveListItem =
  | { kind: "workers-header"; count: number }
  | { kind: "worker"; worker: HiveWorker }
  | { kind: "groups-header"; count: number }
  | { kind: "group"; group: HiveGroup }
  | { kind: "threads-header" }
  | {
      kind: "hive-session";
      session: SessionResponse;
      summary?: HiveSessionSummary;
    }
  | { kind: "active-empty"; label: string }
  | ArchiveListItem;

interface SessionDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onSelectHiveSession: (sessionId: string) => void;
  onNewSession: (type: "chat" | "code") => void;
  onNewHiveSession: () => void;
  onNewSessionWithDir: (path: string) => void;
  onDeleteSession: (
    id: string,
    onDeleted?: () => void,
    onFailed?: () => void,
  ) => void;
  onSetSessionPinned: (id: string, pinned: boolean) => Promise<boolean>;
  onSetSessionArchived: (id: string, archived: boolean) => Promise<boolean>;
  onSetProjectPinned: (ids: string[], pinned: boolean) => Promise<boolean>;
  onSetProjectArchived: (ids: string[], archived: boolean) => Promise<boolean>;
  onDeleteProjectSessions: (
    projectName: string,
    ids: string[],
    onDeleted?: () => void,
    onFailed?: (failedIds: string[]) => void,
  ) => void;
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
  return "Hive threads";
}

function hiveRuntimeLabel(
  summary: HiveSessionSummary | undefined,
  fallbackState: string | null | undefined,
): string | null {
  switch (summary?.runtime?.status ?? fallbackState) {
    case "running":
    case "streaming":
    case "thinking":
    case "tool_executing":
      return "Working";
    case "sleeping":
      return "Sleeping";
    case "paused":
      return "Paused";
    case "awaiting_input":
      return "Needs input";
    case "error":
    case "failed":
      return "Error";
    case "idle":
      return "Ready";
    default:
      return null;
  }
}

export function SessionDrawer({
  isOpen,
  onClose,
  sessions,
  activeSessionId,
  onSelectSession,
  onSelectHiveSession,
  onNewSession,
  onNewHiveSession,
  onNewSessionWithDir,
  onDeleteSession,
  onSetSessionPinned,
  onSetSessionArchived,
  onSetProjectPinned,
  onSetProjectArchived,
  onDeleteProjectSessions,
  onOpenSettings,
  activeMode,
}: SessionDrawerProps) {
  const { theme } = useThemeContext();
  const { client, status } = useConnection();
  const { height: windowHeight } = useWindowDimensions();
  const t = theme.colors;
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [expandedRecentDays, setExpandedRecentDays] =
    useState<Set<string>>(new Set());
  const [codeView, setCodeView] = useState<CodeThreadView>("projects");
  const [threadDensity, setThreadDensity] =
    useState<ThreadDensity>("comfortable");
  const [archiveExpandedMode, setArchiveExpandedMode] =
    useState<SessionType | null>(null);
  const [archivedThreadSessions, setArchivedThreadSessions] = useState<SessionResponse[]>([]);
  const [sessionOverrides, setSessionOverrides] = useState<
    Record<string, SessionListOverride>
  >({});
  const [optimisticSessions, setOptimisticSessions] = useState<SessionResponse[]>([]);
  const [archiveLoading, setArchiveLoading] = useState(false);
  const [projectActivity, setProjectActivity] = useState<
    Record<string, ProjectActivity | null>
  >({});
  const projectActivityCacheRef = useRef<
    Map<string, ProjectActivity | null>
  >(new Map());
  const projectActivityInFlightRef = useRef<Set<string>>(new Set());
  const drawerWasOpenRef = useRef(false);
  const openSwipeableRef = useRef<SwipeableMethods | null>(null);
  const lastSwipeOpenedAtRef = useRef(0);
  const lastAutoExpandedCodeSessionRef = useRef<string | null>(null);
  const lastAutoExpandedRecentDayRef = useRef<string | null>(null);
  const lastAutoExpandedRecentSessionDayRef = useRef<string | null>(null);
  const [hiveSessions, setHiveSessions] = useState<HiveSessionSummary[]>([]);
  const [hiveWorkers, setHiveWorkers] = useState<HiveWorker[]>([]);
  const [hiveGroups, setHiveGroups] = useState<HiveGroup[]>([]);
  // The room mounts only while open; closing tears down its event tail.
  const [openGroupRoomId, setOpenGroupRoomId] = useState<string | null>(null);
  const openingWorkerIdRef = useRef<string | null>(null);

  const pickerProgress = useSharedValue(0);
  const [pickerVisible, setPickerVisible] = useState(false);
  const pickerHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [pickerPath, setPickerPath] = useState("");
  const [pickerParent, setPickerParent] = useState<string | null>(null);
  const [pickerDirs, setPickerDirs] = useState<DirEntry[]>([]);
  const MAX_DIR_CACHE = 40;
  const dirCache = useRef<Map<string, DirCache>>(new Map());
  const setDirCache = (key: string, value: DirCache) => {
    dirCache.current.delete(key);
    dirCache.current.set(key, value);
    while (dirCache.current.size > MAX_DIR_CACHE) {
      const oldest = dirCache.current.keys().next().value;
      if (!oldest) break;
      dirCache.current.delete(oldest);
    }
  };
  const [pickerReady, setPickerReady] = useState(false);
  const pickerHeight = Math.max(300, Math.round(windowHeight * 0.58));
  const archiveExpanded = archiveExpandedMode === activeMode;

  const displaySessions = useMemo(
    () => applySessionListOverrides(sessions, sessionOverrides, optimisticSessions),
    [optimisticSessions, sessionOverrides, sessions],
  );
  const chatSessions = useMemo(
    () => chronologicalSessions(displaySessions, "chat"),
    [displaySessions],
  );
  const codeGroups = useMemo(
    () => codeProjectThreadGroups(displaySessions),
    [displaySessions],
  );
  const recentCodeSessions = useMemo(
    () => chronologicalSessions(displaySessions, "code"),
    [displaySessions],
  );
  const recentCodeDayGroups = useMemo(
    () => chronologicalThreadDayGroups(displaySessions, "code"),
    [displaySessions],
  );
  const visibleArchivedSessions = useMemo(
    () => archivedSessionsForType(archivedThreadSessions, activeMode),
    [activeMode, archivedThreadSessions],
  );
  const archiveListItems = useMemo<ArchiveListItem[]>(() => {
    const items: ArchiveListItem[] = [{ kind: "archive-toggle" }];
    if (!archiveExpanded) return items;
    if (archiveLoading && visibleArchivedSessions.length === 0) {
      items.push({ kind: "archive-loading" });
    } else if (visibleArchivedSessions.length === 0) {
      items.push({ kind: "archive-empty" });
    } else {
      items.push(
        ...visibleArchivedSessions.map((session) => ({
          kind: "archived-session" as const,
          session,
        })),
      );
    }
    return items;
  }, [
    archiveExpanded,
    archiveLoading,
    visibleArchivedSessions,
  ]);

  const chatListItems = useMemo<ChatListItem[]>(
    () => [
      ...(chatSessions.length > 0
        ? chatSessions.map((session) => ({
            kind: "session" as const,
            session,
          }))
        : [{ kind: "active-empty" as const, label: "No Chat threads yet" }]),
      ...archiveListItems,
    ],
    [archiveListItems, chatSessions],
  );

  const hiveSummariesById = useMemo(
    () => new Map(hiveSessions.map((session) => [session.session_id, session])),
    [hiveSessions],
  );
  const activeHiveSessions = useMemo(
    () => chronologicalSessions(displaySessions, "hive"),
    [displaySessions],
  );
  const hiveListItems = useMemo<HiveListItem[]>(
    () => [
      // Workers first: opening one lands in its private DM. Threads keep the
      // existing companion/run session listing below.
      ...(hiveWorkers.length > 0
        ? [
            { kind: "workers-header" as const, count: hiveWorkers.length },
            ...hiveWorkers.map((worker) => ({
              kind: "worker" as const,
              worker,
            })),
          ]
        : []),
      // Groups next: opening one lands in its room as a lightweight surface.
      ...(hiveGroups.length > 0
        ? [
            { kind: "groups-header" as const, count: hiveGroups.length },
            ...hiveGroups.map((group) => ({
              kind: "group" as const,
              group,
            })),
          ]
        : []),
      ...(hiveWorkers.length > 0 || hiveGroups.length > 0
        ? [{ kind: "threads-header" as const }]
        : []),
      ...(activeHiveSessions.length > 0
        ? activeHiveSessions.map((session) => ({
            kind: "hive-session" as const,
            session,
            summary: hiveSummariesById.get(session.id),
          }))
        : [{ kind: "active-empty" as const, label: "No Hive threads yet" }]),
      ...archiveListItems,
    ],
    [
      activeHiveSessions,
      archiveListItems,
      hiveGroups,
      hiveSummariesById,
      hiveWorkers,
    ],
  );

  const codeListItems = useMemo<CodeListItem[]>(() => {
    if (codeView === "recent") {
      return recentCodeDayGroups.flatMap((group) => {
        const items: CodeListItem[] = [{ kind: "day", group }];
        if (expandedRecentDays.has(group.key)) {
          items.push(
            ...group.sessions.map((session) => ({
              kind: "session" as const,
              session,
              directory: sessionProjectDirectory(session),
            })),
          );
        }
        return items;
      });
    }

    const neutralSessions = recentCodeSessions
      .filter((session) => sessionProjectDirectory(session) === "Neutral")
      .map((session) => ({
        kind: "session" as const,
        session,
        directory: "Neutral",
      }));
    const projectItems = codeGroups
      .filter((group) => group.directory !== "Neutral")
      .flatMap((group) => {
        const items: CodeListItem[] = [{ kind: "project", group }];
        if (expandedDirs.has(group.directory)) {
          items.push(
            ...group.sessions.map((session) => ({
              kind: "session" as const,
              session,
              directory: group.directory,
            })),
          );
        }
        return items;
      });
    return [...neutralSessions, ...projectItems];
  }, [
    codeGroups,
    codeView,
    expandedDirs,
    expandedRecentDays,
    recentCodeDayGroups,
    recentCodeSessions,
  ]);
  const codeDisplayItems = useMemo<CodeListItem[]>(
    () => [
      ...(codeListItems.length > 0
        ? codeListItems
        : [{ kind: "active-empty" as const, label: "No Code threads yet" }]),
      ...archiveListItems,
    ],
    [archiveListItems, codeListItems],
  );

  useEffect(() => {
    let active = true;
    void AsyncStorage.getItem(THREAD_DENSITY_STORAGE_KEY).then((stored) => {
      if (active && (stored === "comfortable" || stored === "compact")) {
        setThreadDensity(stored);
      }
    });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (isOpen && !drawerWasOpenRef.current) {
      projectActivityCacheRef.current.clear();
      projectActivityInFlightRef.current.clear();
      setProjectActivity({});
    }
    drawerWasOpenRef.current = isOpen;
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) {
      openSwipeableRef.current?.close();
      openSwipeableRef.current = null;
      return;
    }
  }, [activeMode, isOpen]);

  useEffect(() => {
    if (!isOpen || !archiveExpanded || !client) {
      return;
    }
    let active = true;
    setArchiveLoading(true);
    void client
      .getSessions({ includeArchived: true })
      .then((nextSessions) => {
        if (active) {
          setArchivedThreadSessions(nextSessions);
        }
      })
      .catch(() => {
        if (active) {
          setArchivedThreadSessions([]);
        }
      })
      .finally(() => {
        if (active) {
          setArchiveLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [activeMode, archiveExpanded, client, isOpen]);

  useEffect(() => {
    const liveById = new Map(sessions.map((session) => [session.id, session]));
    setSessionOverrides((current) => {
      let changed = false;
      const next = { ...current };
      for (const [id, override] of Object.entries(current)) {
        const live = liveById.get(id);
        if (override.type === "remove") {
          if (!live) {
            delete next[id];
            changed = true;
          }
          continue;
        }
        if (override.archived_at) {
          if (!live || live.archived_at) {
            delete next[id];
            changed = true;
          }
          continue;
        }
        if (live && !live.archived_at) {
          delete next[id];
          changed = true;
        }
      }
      return changed ? next : current;
    });
    setOptimisticSessions((current) => {
      const next = current.filter((session) => !liveById.has(session.id));
      return next.length === current.length ? current : next;
    });
  }, [sessions]);

  useEffect(() => {
    if (!isOpen || activeMode !== "code" || !client) {
      return;
    }

    const directories =
      codeView === "projects"
        ? codeGroups
            .filter((group) =>
              group.directory !== "Neutral" &&
              expandedDirs.has(group.directory)
            )
            .map((group) => group.directory)
        : Array.from(
            new Set(
              recentCodeSessions
                .slice(0, 8)
                .map((session) => sessionProjectDirectory(session)),
            ),
          );

    for (const directory of directories) {
      if (
        directory === "Neutral" ||
        projectActivityCacheRef.current.has(directory) ||
        projectActivityInFlightRef.current.has(directory)
      ) {
        continue;
      }
      projectActivityInFlightRef.current.add(directory);
      void Promise.all([
        client.getGitStatus(directory),
        client.getGitChanges(directory),
      ])
        .then(([statusResult, changesResult]) => {
          if (!statusResult.in_repo || !changesResult.in_repo) {
            projectActivityCacheRef.current.set(directory, null);
            setProjectActivity((current) => ({ ...current, [directory]: null }));
            return;
          }
          const summary = changesResult.files.reduce(
            (result, file: GitChangedFile) => ({
              additions: result.additions + file.additions,
              deletions: result.deletions + file.deletions,
            }),
            { additions: 0, deletions: 0 },
          );
          const activity: ProjectActivity = {
            branch: statusResult.branch,
            files: changesResult.files.length,
            additions: summary.additions,
            deletions: summary.deletions,
          };
          projectActivityCacheRef.current.set(directory, activity);
          setProjectActivity((current) => ({
            ...current,
            [directory]: activity,
          }));
        })
        .catch(() => {
          projectActivityCacheRef.current.set(directory, null);
          setProjectActivity((current) => ({ ...current, [directory]: null }));
        })
        .finally(() => {
          projectActivityInFlightRef.current.delete(directory);
        });
    }
  }, [
    activeMode,
    client,
    codeGroups,
    codeView,
    expandedDirs,
    isOpen,
    recentCodeSessions,
  ]);

  useEffect(() => {
    if (!isOpen || activeMode !== "hive" || !client) {
      return;
    }
    let active = true;
    void client
      .listHiveSessions()
      .then((nextSessions) => {
        if (active) {
          setHiveSessions(nextSessions);
        }
      })
      .catch(() => {
        if (active) {
          setHiveSessions([]);
        }
      });
    void client
      .listHiveWorkers()
      .then((response) => {
        if (active) {
          setHiveWorkers(response.workers);
        }
      })
      .catch(() => {
        if (active) {
          setHiveWorkers([]);
        }
      });
    void client
      .listHiveGroups()
      .then((response) => {
        if (active) {
          setHiveGroups(response.groups);
        }
      })
      .catch(() => {
        if (active) {
          setHiveGroups([]);
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
      displaySessions,
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
  }, [activeMode, activeSessionId, displaySessions]);

  useEffect(() => {
    if (
      !isOpen ||
      activeMode !== "code" ||
      codeView !== "recent" ||
      recentCodeDayGroups.length === 0
    ) {
      return;
    }

    const keysToExpand: string[] = [];
    const newestDayKey = recentCodeDayGroups[0]?.key;
    if (
      newestDayKey &&
      newestDayKey !== lastAutoExpandedRecentDayRef.current
    ) {
      keysToExpand.push(newestDayKey);
      lastAutoExpandedRecentDayRef.current = newestDayKey;
    }

    const activeDay = activeSessionId
      ? recentCodeDayGroups.find((group) =>
          group.sessions.some((session) => session.id === activeSessionId),
        )
      : undefined;
    const activeSessionDayToken = activeDay && activeSessionId
      ? `${activeSessionId}:${activeDay.key}`
      : null;
    if (
      activeDay &&
      activeSessionDayToken !== lastAutoExpandedRecentSessionDayRef.current
    ) {
      keysToExpand.push(activeDay.key);
      lastAutoExpandedRecentSessionDayRef.current = activeSessionDayToken;
    }

    if (keysToExpand.length === 0) return;
    setExpandedRecentDays((current) => {
      const next = new Set(current);
      for (const key of keysToExpand) next.add(key);
      return next;
    });
  }, [
    activeMode,
    activeSessionId,
    codeView,
    isOpen,
    recentCodeDayGroups,
  ]);

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

  // Directory browse is explicit user cost. Do not preload on every code drawer open.
  // loadPickerRoot still runs from showPicker().

  useEffect(() => {
    if (!isOpen) {
      if (pickerHideTimerRef.current) {
        clearTimeout(pickerHideTimerRef.current);
        pickerHideTimerRef.current = null;
      }
      pickerProgress.value = withTiming(0, { duration: 150 });
      // Fully dismiss the directory slide when threads close so it cannot
      // reappear half-open on the next threads open.
      pickerHideTimerRef.current = setTimeout(() => {
        setPickerVisible(false);
        pickerHideTimerRef.current = null;
      }, 160);
      return () => {
        if (pickerHideTimerRef.current) {
          clearTimeout(pickerHideTimerRef.current);
          pickerHideTimerRef.current = null;
        }
      };
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
    if (pickerHideTimerRef.current) {
      clearTimeout(pickerHideTimerRef.current);
      pickerHideTimerRef.current = null;
    }
    setPickerVisible(true);
    pickerProgress.value = withSpring(1, {
      damping: 20,
      stiffness: 250,
      mass: 0.8,
    });
    void loadPickerRoot();
  }, [loadPickerRoot, pickerProgress]);

  const hidePicker = useCallback(() => {
    if (pickerHideTimerRef.current) {
      clearTimeout(pickerHideTimerRef.current);
      pickerHideTimerRef.current = null;
    }
    pickerProgress.value = withTiming(0, {
      duration: 200,
      easing: Easing.out(Easing.cubic),
    });
    pickerHideTimerRef.current = setTimeout(() => {
      setPickerVisible(false);
      pickerHideTimerRef.current = null;
    }, 210);
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

  const toggleCodeView = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setCodeView((current) => current === "projects" ? "recent" : "projects");
  };

  const toggleThreadDensity = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const next = threadDensity === "comfortable" ? "compact" : "comfortable";
    setThreadDensity(next);
    void AsyncStorage.setItem(THREAD_DENSITY_STORAGE_KEY, next);
  };

  const toggleArchived = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    openSwipeableRef.current?.close();
    openSwipeableRef.current = null;
    setArchiveExpandedMode((current) =>
      current === activeMode ? null : activeMode,
    );
  };

  const hideSessionsLocally = (ids: string[]) => {
    setSessionOverrides((current) => {
      const next = { ...current };
      for (const id of ids) {
        next[id] = { type: "remove" };
      }
      return next;
    });
    setOptimisticSessions((current) =>
      current.filter((session) => !ids.includes(session.id)),
    );
    setArchivedThreadSessions((current) =>
      current.filter((session) => !ids.includes(session.id)),
    );
  };

  const restoreSessionsLocally = (ids: string[]) => {
    setSessionOverrides((current) => {
      const next = { ...current };
      for (const id of ids) {
        delete next[id];
      }
      return next;
    });
  };

  const applyArchiveOverride = (
    session: SessionResponse,
    archived: boolean,
    archivedAt: string | null,
  ) => {
    setSessionOverrides((current) => ({
      ...current,
      [session.id]: { type: "archive", archived_at: archivedAt },
    }));
    if (archived) {
      setOptimisticSessions((current) =>
        current.filter((candidate) => candidate.id !== session.id),
      );
      setArchivedThreadSessions((current) => [
        { ...session, archived_at: archivedAt ?? new Date().toISOString() },
        ...current.filter((candidate) => candidate.id !== session.id),
      ]);
      return;
    }
    setOptimisticSessions((current) => [
      { ...session, archived_at: null },
      ...current.filter((candidate) => candidate.id !== session.id),
    ]);
    setArchivedThreadSessions((current) =>
      current.filter((candidate) => candidate.id !== session.id),
    );
  };

  const runArchiveChange = async (
    session: SessionResponse,
    archived: boolean,
    swipeable?: SwipeableMethods,
  ) => {
    swipeable?.close();
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const previousArchived = archivedThreadSessions;
    const previousOverrides = sessionOverrides;
    const previousOptimistic = optimisticSessions;
    const archivedAt = archived
      ? session.archived_at ?? new Date().toISOString()
      : null;
    applyArchiveOverride(session, archived, archivedAt);
    const changed = await onSetSessionArchived(session.id, archived);
    if (!changed) {
      setArchivedThreadSessions(previousArchived);
      setSessionOverrides(previousOverrides);
      setOptimisticSessions(previousOptimistic);
      return;
    }
    if (archiveExpanded && client) {
      void client
        .getSessions({ includeArchived: true })
        .then(setArchivedThreadSessions)
        .catch(() => {
          // Keep the immediately updated local row when a refresh is unavailable.
        });
    }
  };

  const runPinChange = async (
    session: SessionResponse,
    pinned: boolean,
    swipeable?: SwipeableMethods,
  ) => {
    swipeable?.close();
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    await onSetSessionPinned(session.id, pinned);
  };

  const runProjectPinChange = async (
    group: CodeProjectThreadGroup,
    pinned: boolean,
    swipeable?: SwipeableMethods,
  ) => {
    swipeable?.close();
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    await onSetProjectPinned(group.sessions.map((session) => session.id), pinned);
  };

  const runProjectArchiveChange = async (
    group: CodeProjectThreadGroup,
    archived: boolean,
    swipeable?: SwipeableMethods,
  ) => {
    swipeable?.close();
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    const previousArchived = archivedThreadSessions;
    const previousOverrides = sessionOverrides;
    const previousOptimistic = optimisticSessions;
    const archivedAt = archived ? new Date().toISOString() : null;
    for (const session of group.sessions) {
      applyArchiveOverride(session, archived, archivedAt);
    }
    const changed = await onSetProjectArchived(
      group.sessions.map((session) => session.id),
      archived,
    );
    if (!changed) {
      setArchivedThreadSessions(previousArchived);
      setSessionOverrides(previousOverrides);
      setOptimisticSessions(previousOptimistic);
    }
  };

  const swipeAction = (
    label: string,
    color: string,
    icon: ReactNode,
    onPress: () => void,
  ) => (
    <GesturePressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={[styles.swipeAction, { backgroundColor: color }]}
    >
      {icon}
      <Text style={styles.swipeActionLabel}>{label}</Text>
    </GesturePressable>
  );

  const renderBotAvatar = (session: SessionResponse, active: boolean) => {
    const provider = sessionProviderKey(session);
    const label = sessionProviderLabel(session);
    const color = provider === "openai"
      ? t.success
      : provider === "anthropic"
        ? t.warning
        : provider === "minimax" || provider === "openrouter"
          ? t.userMessage
          : active
            ? t.userMessage
            : t.foreground;
    return (
      <View
        accessibilityLabel={`${label} bot`}
        style={[
          styles.botAvatar,
          {
            backgroundColor: `${color}14`,
            borderColor: `${color}24`,
          },
        ]}
      >
        <Bot size={17} color={color} strokeWidth={1.9} />
      </View>
    );
  };

  const renderSession = (
    session: SessionResponse,
    options?: {
      directory?: string;
      showProject?: boolean;
      hiveSummary?: HiveSessionSummary;
    },
  ) => {
    const active = session.id === activeSessionId;
    const directory = options?.directory ?? sessionProjectDirectory(session);
    const activity = projectActivity[directory];
    const branch = session.target_branch ?? activity?.branch ?? null;
    const modelLabel = sessionModelLabel(
      session.model_key?.model_id ?? session.model,
    );
    const isHive = session.session_type === "hive";
    const stateLabel = isHive
      ? hiveRuntimeLabel(options?.hiveSummary, session.agent_state)
      : sessionStateLabel(session.agent_state);
    const compact = threadDensity === "compact";
    const projectLabel = options?.showProject ? dirDisplayName(directory) : null;
    const primaryMeta = isHive
      ? [
          options?.hiveSummary?.runtime?.crew_slug || "Hive Worker",
          modelLabel ?? sessionProviderLabel(session),
        ].join(" · ")
      : [projectLabel, branch, modelLabel].filter(Boolean).join(" · ");
    const hasActivity = !isHive && activity !== undefined && activity !== null;
    const activityLabel = isHive
      ? options?.hiveSummary?.runtime?.sleep_reason ?? null
      : hasActivity
      ? activity.files === 0
        ? "Clean workspace"
        : `${activity.files} ${activity.files === 1 ? "file" : "files"}`
      : null;
    const statusColor =
      stateLabel === "Working"
        ? t.success
        : stateLabel === "Needs input"
          ? t.warning
          : stateLabel === "Error"
            ? t.error
            : t.mutedForeground;
    const archived = Boolean(session.archived_at);
    const pinned = Boolean(session.pinned_at);
    const swipeableRef = createRef<SwipeableMethods>();

    return (
      <ReanimatedSwipeable
        key={session.id}
        ref={swipeableRef}
        friction={2}
        leftThreshold={34}
        rightThreshold={52}
        dragOffsetFromLeftEdge={12}
        dragOffsetFromRightEdge={12}
        overshootLeft={false}
        overshootRight={false}
        enableTrackpadTwoFingerGesture
        containerStyle={styles.swipeContainer}
        childrenContainerStyle={{ backgroundColor: t.background }}
        onSwipeableWillOpen={() => {
          lastSwipeOpenedAtRef.current = Date.now();
          const next = swipeableRef.current;
          if (openSwipeableRef.current && openSwipeableRef.current !== next) {
            openSwipeableRef.current.close();
          }
          openSwipeableRef.current = next;
        }}
        onSwipeableOpen={() => {
          lastSwipeOpenedAtRef.current = Date.now();
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        }}
        onSwipeableClose={() => {
          if (openSwipeableRef.current === swipeableRef.current) {
            openSwipeableRef.current = null;
          }
        }}
        renderLeftActions={(_progress, _translation, methods) => (
          <View style={styles.swipeActionGroup}>
            {archived
              ? swipeAction(
                  "Restore",
                  t.success,
                  <ArchiveRestore size={19} color={t.onAccent} strokeWidth={2} />,
                  () => void runArchiveChange(session, false, methods),
                )
              : swipeAction(
                  pinned ? "Unpin" : "Pin",
                  t.userMessage,
                  pinned
                    ? <PinOff size={19} color={t.onAccent} strokeWidth={2} />
                    : <Pin size={19} color={t.onAccent} strokeWidth={2} />,
                  () => void runPinChange(session, !pinned, methods),
                )}
          </View>
        )}
        renderRightActions={(_progress, _translation, methods) => (
          <View style={styles.swipeActionGroup}>
            {!archived
              ? swipeAction(
                  "Archive",
                  t.warning,
                  <Archive size={19} color={t.onAccent} strokeWidth={2} />,
                  () => void runArchiveChange(session, true, methods),
                )
              : null}
            {swipeAction(
              "Delete",
              t.error,
              <Trash2 size={19} color={t.onAccent} strokeWidth={2} />,
              () => {
                methods.close();
                onDeleteSession(
                  session.id,
                  () => hideSessionsLocally([session.id]),
                  () => restoreSessionsLocally([session.id]),
                );
              },
            )}
          </View>
        )}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ selected: active }}
          accessibilityActions={[
            {
              name: archived ? "restore" : "togglePin",
              label: archived ? "Restore" : pinned ? "Unpin" : "Pin",
            },
            ...(!archived
              ? [{ name: "archive", label: "Archive" }]
              : []),
            { name: "delete", label: "Delete" },
          ]}
          onAccessibilityAction={(event) => {
            switch (event.nativeEvent.actionName) {
              case "restore":
                void runArchiveChange(session, false);
                break;
              case "togglePin":
                void runPinChange(session, !pinned);
                break;
              case "archive":
                void runArchiveChange(session, true);
                break;
              case "delete":
                onDeleteSession(
                  session.id,
                  () => hideSessionsLocally([session.id]),
                  () => restoreSessionsLocally([session.id]),
                );
                break;
            }
          }}
          onPress={() => {
            if (openSwipeableRef.current === swipeableRef.current) {
              // Web may emit a Pressable click for the pointer-up that opened
              // the swipe tray. Do not interpret that same release as a
              // request to close it; a later deliberate row tap can close.
              if (Date.now() - lastSwipeOpenedAtRef.current < 350) {
                return;
              }
              swipeableRef.current?.close();
              return;
            }
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            if (isHive) {
              onSelectHiveSession(session.id);
            } else {
              onSelectSession(session);
            }
          }}
          style={[
            styles.sessionItem,
            compact && styles.sessionItemCompact,
            styles.swipeSessionItem,
            { backgroundColor: t.background },
            active && { backgroundColor: `${t.userMessage}12` },
          ]}
        >
          <View style={styles.sessionRow}>
          {isHive ? renderBotAvatar(session, active) : null}
          <View style={styles.sessionCopy}>
          <View style={styles.sessionTitleRow}>
          <Text
            numberOfLines={compact ? 1 : 2}
            style={[
              styles.sessionTitle,
              compact && styles.sessionTitleCompact,
              { color: active ? t.userMessage : t.foreground },
            ]}
          >
            {session.title || "Untitled"}
          </Text>
          {pinned ? (
            <Pin size={13} color={t.userMessage} strokeWidth={2.2} />
          ) : null}
          {compact ? (
            <Text
              style={[styles.sessionTime, { color: t.mutedForeground }]}
            >
              {formatTime(session.updated_at)}
            </Text>
          ) : null}
          </View>
        {compact ? (
          <View style={[styles.sessionMeta, styles.sessionMetaCompact]}>
            {stateLabel ? (
              <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
            ) : null}
            <Text
              numberOfLines={1}
              style={[styles.sessionModel, { color: t.mutedForeground }]}
            >
              {[primaryMeta, activityLabel].filter(Boolean).join(" · ") ||
                (isHive ? "Hive Worker" : session.session_type === "chat" ? "Agent" : "Code task")}
            </Text>
            {hasActivity && activity.additions > 0 ? (
              <Text style={[styles.changeStat, { color: t.success }]}>+{formatThreadMetric(activity.additions)}</Text>
            ) : null}
            {hasActivity && activity.deletions > 0 ? (
              <Text style={[styles.changeStat, { color: t.error }]}>−{formatThreadMetric(activity.deletions)}</Text>
            ) : null}
          </View>
        ) : (
          <>
            <View style={styles.sessionMeta}>
              <Text
                numberOfLines={1}
                style={[styles.sessionModel, { color: t.mutedForeground }]}
              >
                {primaryMeta ||
                  (isHive ? "Hive Worker" : session.session_type === "chat" ? "Agent" : "Code task")}
              </Text>
              <Text
                style={[styles.sessionTime, { color: t.mutedForeground }]}
              >
                {formatTime(session.updated_at)}
              </Text>
            </View>
            <View style={styles.activityMeta}>
              {activityLabel ? (
                <Text
                  style={[styles.activityLabel, { color: t.mutedForeground }]}
                >
                  {activityLabel}
                </Text>
              ) : (
                <Text
                  style={[styles.activityLabel, { color: t.mutedForeground }]}
                >
                  {isHive
                    ? "No active schedule"
                    : directory === "Neutral"
                      ? "No workspace"
                      : "Workspace details unavailable"}
                </Text>
              )}
              {hasActivity && activity.additions > 0 ? (
                <Text style={[styles.changeStat, { color: t.success }]}>+{activity.additions}</Text>
              ) : null}
              {hasActivity && activity.deletions > 0 ? (
                <Text style={[styles.changeStat, { color: t.error }]}>−{activity.deletions}</Text>
              ) : null}
              <View style={styles.spacer} />
              {stateLabel ? (
                <View style={styles.stateLabel}>
                  <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
                  <Text style={[styles.stateText, { color: statusColor }]}>{stateLabel}</Text>
                </View>
              ) : null}
            </View>
          </>
        )}
          </View>
          </View>
        </Pressable>
      </ReanimatedSwipeable>
    );
  };

  const renderArchiveListItem = (item: ArchiveListItem) => {
    switch (item.kind) {
      case "archive-toggle":
        return (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`${archiveExpanded ? "Hide" : "Show"} archived ${activeMode} conversations`}
            accessibilityState={{ expanded: archiveExpanded }}
            onPress={toggleArchived}
            style={styles.archiveToggle}
          >
            <View
              style={[
                styles.archiveIcon,
                { backgroundColor: `${t.mutedForeground}0C` },
              ]}
            >
              <Archive size={17} color={t.mutedForeground} strokeWidth={1.8} />
            </View>
            <Text style={[styles.archiveLabel, { color: t.mutedForeground }]}>Archived</Text>
            {archiveExpanded && !archiveLoading ? (
              <Text style={[styles.archiveCount, { color: t.mutedForeground }]}>{visibleArchivedSessions.length}</Text>
            ) : null}
            <View style={styles.spacer} />
            {archiveExpanded ? (
              <ChevronDown size={17} color={t.mutedForeground} strokeWidth={1.8} />
            ) : (
              <ChevronRight size={17} color={t.mutedForeground} strokeWidth={1.8} />
            )}
          </Pressable>
        );
      case "archived-session":
        return renderSession(item.session, {
          showProject: activeMode === "code",
        });
      case "archive-loading":
        return (
          <View style={styles.archiveMessage}>
            <SessionListSkeleton count={2} />
          </View>
        );
      case "archive-empty":
        return (
          <Text style={[styles.archiveEmptyText, { color: t.mutedForeground }]}>No archived conversations</Text>
        );
    }
  };

  const renderChatListItem = ({ item }: { item: ChatListItem }) => {
    if (item.kind === "session") return renderSession(item.session);
    if (item.kind === "active-empty") {
      return (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>{item.label}</Text>
      );
    }
    return renderArchiveListItem(item);
  };

  const openWorkerDm = async (worker: HiveWorker) => {
    // A bound DM opens instantly; an unbound Worker ensures its DM first.
    if (worker.dm_session_id) {
      onSelectHiveSession(worker.dm_session_id);
      return;
    }
    if (!client || openingWorkerIdRef.current) {
      return;
    }
    openingWorkerIdRef.current = worker.id;
    try {
      const dm = await client.ensureHiveWorkerDm(worker.id);
      setHiveWorkers((current) =>
        current.map((candidate) =>
          candidate.id === worker.id
            ? { ...candidate, dm_session_id: dm.session_id }
            : candidate,
        ),
      );
      onSelectHiveSession(dm.session_id);
    } catch {
      // The drawer stays open; the Worker row remains tappable to retry.
    } finally {
      openingWorkerIdRef.current = null;
    }
  };

  const renderWorkerRow = (worker: HiveWorker) => {
    const color = workerAvatarColor(worker);
    const paused = worker.status === "paused";
    const working = worker.dm_agent_state === "running";
    const statusColor = paused
      ? t.warning
      : working
        ? t.success
        : t.mutedForeground;

    return (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Open DM with Worker ${worker.display_name}`}
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          void openWorkerDm(worker);
        }}
        style={[styles.sessionItem, { backgroundColor: t.background }]}
      >
        <View style={styles.sessionRow}>
          <View
            style={[
              styles.workerAvatar,
              { backgroundColor: `${color}22`, borderColor: `${color}55` },
            ]}
          >
            <Text style={[styles.workerAvatarText, { color }]}>
              {workerInitials(worker.display_name)}
            </Text>
          </View>
          <View style={styles.sessionCopy}>
            <View style={styles.sessionTitleRow}>
              <Text
                numberOfLines={1}
                style={[styles.sessionTitle, { color: t.foreground }]}
              >
                {worker.display_name}
              </Text>
            </View>
            <View style={styles.sessionMeta}>
              <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
              <Text
                numberOfLines={1}
                style={[styles.sessionModel, { color: t.mutedForeground }]}
              >
                {paused ? `Paused · ${workerMetaLine(worker)}` : workerMetaLine(worker)}
              </Text>
            </View>
          </View>
        </View>
      </Pressable>
    );
  };

  const renderGroupRow = (group: HiveGroup) => {
    const memberCount = group.members.length;
    const active = Boolean(group.active_turn_id);
    return (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Open group ${group.title}`}
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          setOpenGroupRoomId(group.id);
        }}
        style={[styles.sessionItem, { backgroundColor: t.background }]}
      >
        <View style={styles.sessionRow}>
          <View
            style={[
              styles.workerAvatar,
              {
                backgroundColor: `${t.mutedForeground}14`,
                borderColor: `${t.mutedForeground}33`,
              },
            ]}
          >
            <Users size={15} color={t.mutedForeground} strokeWidth={1.8} />
          </View>
          <View style={styles.sessionCopy}>
            <View style={styles.sessionTitleRow}>
              <Text
                numberOfLines={1}
                style={[styles.sessionTitle, { color: t.foreground }]}
              >
                {group.title}
              </Text>
            </View>
            <View style={styles.sessionMeta}>
              <View
                style={[
                  styles.statusDot,
                  { backgroundColor: active ? t.success : t.mutedForeground },
                ]}
              />
              <Text
                numberOfLines={1}
                style={[styles.sessionModel, { color: t.mutedForeground }]}
              >
                {memberCount} Worker{memberCount === 1 ? "" : "s"}
                {active ? " · turn running" : ""}
              </Text>
            </View>
          </View>
        </View>
      </Pressable>
    );
  };

  const renderHiveListItem = ({ item }: { item: HiveListItem }) => {
    if (item.kind === "workers-header") {
      return (
        <Text style={[styles.hiveSectionHeader, { color: t.mutedForeground }]}>
          Workers · {item.count}
        </Text>
      );
    }
    if (item.kind === "worker") {
      return renderWorkerRow(item.worker);
    }
    if (item.kind === "threads-header") {
      return (
        <Text style={[styles.hiveSectionHeader, { color: t.mutedForeground }]}>
          Threads
        </Text>
      );
    }
    if (item.kind === "hive-session") {
      return renderSession(item.session, { hiveSummary: item.summary });
    }
    if (item.kind === "groups-header") {
      return (
        <Text style={[styles.hiveSectionHeader, { color: t.mutedForeground }]}>
          Groups · {item.count}
        </Text>
      );
    }
    if (item.kind === "group") {
      return renderGroupRow(item.group);
    }
    if (item.kind === "active-empty") {
      return (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>{item.label}</Text>
      );
    }
    return renderArchiveListItem(item);
  };

  const renderCodeListItem = ({ item }: { item: CodeListItem }) => {
    if (
      item.kind === "archive-toggle" ||
      item.kind === "archived-session" ||
      item.kind === "archive-loading" ||
      item.kind === "archive-empty"
    ) {
      return renderArchiveListItem(item);
    }
    if (item.kind === "active-empty") {
      return (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>{item.label}</Text>
      );
    }
    if (item.kind === "session") {
      return renderSession(item.session, {
        directory: item.directory,
        showProject: codeView === "recent",
      });
    }
    if (item.kind === "day") {
      const expanded = expandedRecentDays.has(item.group.key);
      return (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${expanded ? "Collapse" : "Expand"} ${item.group.label} Code tasks`}
          accessibilityState={{ expanded }}
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setExpandedRecentDays((current) => {
              const next = new Set(current);
              if (next.has(item.group.key)) {
                next.delete(item.group.key);
              } else {
                next.add(item.group.key);
              }
              return next;
            });
          }}
          style={styles.dayHeader}
        >
          <Text style={[styles.dayLabel, { color: t.mutedForeground }]}>
            {item.group.label}
          </Text>
          <View style={styles.spacer} />
          {expanded ? (
            <ChevronDown size={16} color={t.mutedForeground} strokeWidth={1.8} />
          ) : (
            <ChevronRight size={16} color={t.mutedForeground} strokeWidth={1.8} />
          )}
        </Pressable>
      );
    }

    const { group } = item;
    const expanded = expandedDirs.has(group.directory);
    const pinned = group.pinnedAt > 0;
    const swipeableRef = createRef<SwipeableMethods>();
    const sessionIds = group.sessions.map((session) => session.id);
    return (
      <ReanimatedSwipeable
        key={`project:${group.directory}`}
        ref={swipeableRef}
        friction={2}
        leftThreshold={34}
        rightThreshold={52}
        dragOffsetFromLeftEdge={12}
        dragOffsetFromRightEdge={12}
        overshootLeft={false}
        overshootRight={false}
        enableTrackpadTwoFingerGesture
        containerStyle={styles.projectSwipeContainer}
        childrenContainerStyle={{ backgroundColor: t.background }}
        onSwipeableWillOpen={() => {
          lastSwipeOpenedAtRef.current = Date.now();
          const next = swipeableRef.current;
          if (openSwipeableRef.current && openSwipeableRef.current !== next) {
            openSwipeableRef.current.close();
          }
          openSwipeableRef.current = next;
        }}
        onSwipeableOpen={() => {
          lastSwipeOpenedAtRef.current = Date.now();
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        }}
        onSwipeableClose={() => {
          if (openSwipeableRef.current === swipeableRef.current) {
            openSwipeableRef.current = null;
          }
        }}
        renderLeftActions={(_progress, _translation, methods) => (
          <View style={styles.swipeActionGroup}>
            {swipeAction(
              pinned ? "Unpin" : "Pin",
              t.userMessage,
              pinned
                ? <PinOff size={19} color={t.onAccent} strokeWidth={2} />
                : <Pin size={19} color={t.onAccent} strokeWidth={2} />,
              () => void runProjectPinChange(group, !pinned, methods),
            )}
          </View>
        )}
        renderRightActions={(_progress, _translation, methods) => (
          <View style={styles.swipeActionGroup}>
            {swipeAction(
              "Archive",
              t.warning,
              <Archive size={19} color={t.onAccent} strokeWidth={2} />,
              () => void runProjectArchiveChange(group, true, methods),
            )}
            {swipeAction(
              "Delete",
              t.error,
              <Trash2 size={19} color={t.onAccent} strokeWidth={2} />,
              () => {
                methods.close();
                onDeleteProjectSessions(
                  dirDisplayName(group.directory),
                  sessionIds,
                  () => hideSessionsLocally(sessionIds),
                  restoreSessionsLocally,
                );
              },
            )}
          </View>
        )}
      >
        <View
          style={[
            styles.dirHeader,
            threadDensity === "compact" && styles.dirHeaderCompact,
            { backgroundColor: t.background },
          ]}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ expanded }}
            accessibilityLabel={`${expanded ? "Collapse" : "Expand"} ${dirDisplayName(group.directory)}`}
            accessibilityActions={[
              { name: "togglePin", label: pinned ? "Unpin project" : "Pin project" },
              { name: "archive", label: "Archive project conversations" },
              { name: "delete", label: "Delete project conversations" },
            ]}
            onAccessibilityAction={(event) => {
              switch (event.nativeEvent.actionName) {
                case "togglePin":
                  void runProjectPinChange(group, !pinned);
                  break;
                case "archive":
                  void runProjectArchiveChange(group, true);
                  break;
                case "delete":
                  onDeleteProjectSessions(
                    dirDisplayName(group.directory),
                    sessionIds,
                    () => hideSessionsLocally(sessionIds),
                    restoreSessionsLocally,
                  );
                  break;
              }
            }}
            onPress={() => {
              if (openSwipeableRef.current === swipeableRef.current) {
                if (Date.now() - lastSwipeOpenedAtRef.current < 350) return;
                swipeableRef.current?.close();
                return;
              }
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
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
            style={styles.dirToggle}
          >
            {expanded ? (
              <ChevronDown size={16} color={t.mutedForeground} />
            ) : (
              <ChevronRight size={16} color={t.mutedForeground} />
            )}
            {expanded ? (
              <FolderOpen size={18} color={t.thinking} strokeWidth={1.7} />
            ) : (
              <Folder size={18} color={t.mutedForeground} strokeWidth={1.6} />
            )}
            <Text
              numberOfLines={1}
              style={[
                styles.dirName,
                { color: expanded ? t.foreground : t.mutedForeground },
              ]}
            >
              {dirDisplayName(group.directory)}
            </Text>
            {pinned ? (
              <Pin size={13} color={t.userMessage} strokeWidth={2.2} />
            ) : null}
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`New Code task in ${dirDisplayName(group.directory)}`}
            hitSlop={8}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
              onNewSessionWithDir(group.directory);
            }}
            style={styles.projectAddButton}
          >
            <Plus size={17} color={t.mutedForeground} strokeWidth={1.8} />
          </Pressable>
        </View>
      </ReanimatedSwipeable>
    );
  };

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
        accessibilityLabel={`New ${activeMode} thread`}
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
          if (activeMode === "hive") {
            onNewHiveSession();
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
      // Threads + directory picker should fully unmount when closed.
      retainContent={false}
    >
      <View style={styles.content}>
        {activeMode === "code" ? (
          <View
            style={[styles.threadControls, { borderBottomColor: t.border }]}
          >
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={codeView === "projects"
                ? "Show recent Code tasks"
                : "Group Code tasks by project"}
              accessibilityValue={{
                text: codeView === "projects" ? "Projects" : "Recent",
              }}
              onPress={toggleCodeView}
              style={[
                styles.threadControlButton,
                { backgroundColor: `${t.mutedForeground}0C` },
              ]}
            >
              {codeView === "projects" ? (
                <FolderTree size={19} color={t.foreground} strokeWidth={1.8} />
              ) : (
                <Clock3 size={19} color={t.foreground} strokeWidth={1.8} />
              )}
            </Pressable>

            <Pressable
              accessibilityRole="button"
              accessibilityLabel={threadDensity === "comfortable"
                ? "Use compact task list"
                : "Use detailed task list"}
              accessibilityValue={{
                text: threadDensity === "comfortable" ? "Detailed" : "Compact",
              }}
              onPress={toggleThreadDensity}
              style={[
                styles.threadControlButton,
                { backgroundColor: `${t.mutedForeground}0C` },
              ]}
            >
              {threadDensity === "comfortable" ? (
                <Rows3 size={19} color={t.foreground} strokeWidth={1.8} />
              ) : (
                <List size={19} color={t.foreground} strokeWidth={1.8} />
              )}
            </Pressable>
          </View>
        ) : null}
        {activeMode === "chat" ? (
          <FlatList
            style={styles.list}
            contentContainerStyle={styles.listContent}
            data={chatListItems}
            keyExtractor={(item) => {
              if (item.kind === "session") return `session:${item.session.id}`;
              if (item.kind === "archived-session") return `archived:${item.session.id}`;
              return item.kind;
            }}
            extraData={displaySessions}
            renderItem={renderChatListItem}
            windowSize={7}
            maxToRenderPerBatch={10}
            initialNumToRender={14}
            removeClippedSubviews={false}
            showsVerticalScrollIndicator={false}
          />
        ) : activeMode === "hive" ? (
          openGroupRoomId ? (
            <HiveGroupRoomView
              groupId={openGroupRoomId}
              onBack={() => setOpenGroupRoomId(null)}
            />
          ) : (
            <FlatList
              style={styles.list}
              contentContainerStyle={styles.listContent}
              data={hiveListItems}
              keyExtractor={(item) => {
                if (item.kind === "worker") return `worker:${item.worker.id}`;
                if (item.kind === "group") return `group:${item.group.id}`;
                if (item.kind === "hive-session") return `hive:${item.session.id}`;
                if (item.kind === "archived-session") return `archived:${item.session.id}`;
                return item.kind;
              }}
              extraData={displaySessions}
              renderItem={renderHiveListItem}
              windowSize={7}
              maxToRenderPerBatch={10}
              initialNumToRender={14}
              removeClippedSubviews={false}
              showsVerticalScrollIndicator={false}
            />
          )
        ) : (
          <FlatList
            style={styles.list}
            contentContainerStyle={styles.listContent}
            data={codeDisplayItems}
            keyExtractor={(item) => {
              if (item.kind === "project") return `project:${item.group.directory}`;
              if (item.kind === "day") return `day:${item.group.key}`;
              if (item.kind === "session") return `session:${item.session.id}`;
              if (item.kind === "archived-session") return `archived:${item.session.id}`;
              return item.kind;
            }}
            extraData={displaySessions}
            renderItem={renderCodeListItem}
            windowSize={7}
            maxToRenderPerBatch={12}
            initialNumToRender={16}
            removeClippedSubviews={false}
            showsVerticalScrollIndicator={false}
          />
        )}

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
  list: {
    flex: 1,
  },
  listContent: {
    paddingHorizontal: 12,
    paddingTop: 4,
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
  sessionItemCompact: {
    paddingVertical: 7,
    borderRadius: 9,
    marginBottom: 0,
  },
  sessionRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
  },
  sessionCopy: {
    flex: 1,
    minWidth: 0,
  },
  botAvatar: {
    width: 32,
    height: 32,
    marginTop: 1,
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  swipeSessionItem: {
    marginBottom: 0,
  },
  swipeContainer: {
    marginBottom: 3,
    borderRadius: 12,
    overflow: "hidden",
  },
  projectSwipeContainer: {
    borderRadius: 10,
    overflow: "hidden",
  },
  swipeActionGroup: {
    flexDirection: "row",
    height: "100%",
  },
  swipeAction: {
    width: 76,
    height: "100%",
    alignItems: "center",
    justifyContent: "center",
    gap: 4,
  },
  swipeActionLabel: {
    color: colors.onAccent,
    fontSize: 10,
    fontWeight: "700",
  },
  sessionTitleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  sessionTitle: {
    flex: 1,
    minWidth: 0,
    fontSize: 14,
    fontWeight: "600",
    lineHeight: 19,
  },
  sessionTitleCompact: {
    fontSize: 13,
    lineHeight: 17,
  },
  sessionMeta: {
    marginTop: 5,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  sessionMetaCompact: {
    marginTop: 2,
    gap: 5,
  },
  sessionTime: {
    fontSize: 11,
  },
  sessionModel: {
    flex: 1,
    fontSize: 11,
  },
  activityMeta: {
    minHeight: 16,
    marginTop: 3,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  activityLabel: {
    fontSize: 11,
  },
  changeStat: {
    fontSize: 11,
    fontWeight: "700",
  },
  stateLabel: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
  },
  stateText: {
    fontSize: 10,
    fontWeight: "600",
  },
  statusDot: {
    width: 7,
    height: 7,
    borderRadius: 4,
  },
  hiveSectionHeader: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
    paddingHorizontal: 12,
    paddingTop: 12,
    paddingBottom: 4,
  },
  workerAvatar: {
    width: 30,
    height: 30,
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  workerAvatarText: {
    fontSize: 12,
    fontWeight: "700",
  },
  dirHeader: {
    minHeight: 50,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 1,
    paddingRight: 4,
  },
  dirHeaderCompact: {
    minHeight: 42,
  },
  dirToggle: {
    minHeight: 42,
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 5,
  },
  dirName: {
    flex: 1,
    minWidth: 0,
    fontSize: 14,
    fontWeight: "700",
  },
  dayHeader: {
    minHeight: 42,
    marginTop: 4,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  dayLabel: {
    fontSize: 13,
    fontWeight: "700",
  },
  projectAddButton: {
    width: 38,
    height: 38,
    alignItems: "center",
    justifyContent: "center",
  },
  threadControls: {
    minHeight: 52,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  threadControlButton: {
    width: 36,
    height: 36,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  archiveToggle: {
    minHeight: 48,
    marginTop: 18,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  archiveIcon: {
    width: 32,
    height: 32,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  archiveLabel: {
    fontSize: 13,
    fontWeight: "600",
  },
  archiveCount: {
    fontSize: 11,
    fontWeight: "600",
  },
  archiveMessage: {
    paddingVertical: 6,
  },
  archiveEmptyText: {
    paddingVertical: 18,
    textAlign: "center",
    fontSize: 12,
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
