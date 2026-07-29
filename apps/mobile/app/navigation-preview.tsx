import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import {
  Animated,
  PanResponder,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import {
  Archive,
  ArrowDown,
  Blocks,
  CalendarClock,
  Check,
  ChevronRight,
  CircleDot,
  Clock3,
  Code2,
  FileCode2,
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  Globe2,
  History,
  LayoutGrid,
  MemoryStick,
  MessageCircle,
  MessagesSquare,
  MoreHorizontal,
  Paperclip,
  Plus,
  Search,
  Send,
  Settings,
  Sparkles,
  SquarePlus,
  TerminalSquare,
  Toolbox,
  Wifi,
  Workflow,
  type LucideIcon,
} from "lucide-react-native";

import { useThemeContext } from "../hooks/useTheme";
import { HiveIcon } from "../components/brand";

type Mode = "chat" | "code" | "mako";
type SheetKind = "threads" | "toolbox" | null;
type ThemeColors = ReturnType<typeof useThemeContext>["theme"]["colors"];

type ModeIcon = ComponentType<{
  size?: number;
  color?: string;
  strokeWidth?: number;
}>;

const MODES: Array<{ id: Mode; label: string; icon: ModeIcon }> = [
  { id: "chat", label: "Chat", icon: MessageCircle },
  { id: "code", label: "Code", icon: Code2 },
  { id: "mako", label: "Hive", icon: HiveIcon },
];

const MODE_INDEX: Record<Mode, number> = { chat: 0, code: 1, mako: 2 };

const TOOL_TABS: Record<
  Mode,
  Array<{ id: string; label: string; icon: LucideIcon }>
> = {
  chat: [
    { id: "artifacts", label: "Artifacts", icon: LayoutGrid },
    { id: "plugins", label: "Plugins", icon: Blocks },
    { id: "history", label: "Library", icon: Archive },
  ],
  code: [
    { id: "browser", label: "Browser", icon: Globe2 },
    { id: "terminal", label: "Terminal", icon: TerminalSquare },
    { id: "changes", label: "Changes", icon: FileCode2 },
  ],
  mako: [
    { id: "schedule", label: "Schedule", icon: CalendarClock },
    { id: "runs", label: "Runs", icon: Workflow },
    { id: "memory", label: "Memory", icon: MemoryStick },
  ],
};

const THREADS: Record<
  Mode,
  Array<{ title: string; detail: string; time: string; active?: boolean }>
> = {
  chat: [
    {
      title: "Mobile navigation system",
      detail: "18 messages · GPT 5.6",
      time: "now",
      active: true,
    },
    {
      title: "Provider research",
      detail: "Deep research",
      time: "2h",
    },
    {
      title: "Release notes draft",
      detail: "7 messages",
      time: "1d",
    },
    {
      title: "Designing the artifact system",
      detail: "24 messages · GPT 5.6",
      time: "3d",
    },
  ],
  code: [
    {
      title: "Native bottom sheets",
      detail: "Mitsuro · codex/navigation",
      time: "now",
      active: true,
    },
    {
      title: "Hive scheduler polish",
      detail: "Mitsuro · codex/mako-ui",
      time: "3h",
    },
    {
      title: "Tool output renderer",
      detail: "Mitsuro · main",
      time: "2d",
    },
  ],
  mako: [
    {
      title: "Release Captain",
      detail: "Releases and deployment · active",
      time: "live",
      active: true,
    },
    {
      title: "Product Researcher",
      detail: "Research and long-form reports",
      time: "8h",
    },
    {
      title: "Repo Caretaker",
      detail: "Maintenance and dependency health",
      time: "2d",
    },
  ],
};

const CODE_PROJECTS = [
  {
    name: "Mitsuro",
    path: "/Users/Jacob/Documents/Krusty",
    time: "now",
    threads: THREADS.code,
  },
  {
    name: "Bullring",
    path: "/Users/Jacob/Documents/Sol-Dev",
    time: "2d",
    threads: [
      {
        title: "Mobile launch polish",
        detail: "codex/mobile-polish",
        time: "2d",
      },
      {
        title: "Community Wars",
        detail: "main",
        time: "5d",
      },
    ],
  },
];

function ModeIsland({
  mode,
  onChange,
  colors,
}: {
  mode: Mode;
  onChange: (mode: Mode) => void;
  colors: ThemeColors;
}) {
  return (
    <View
      style={[
        styles.modeIsland,
        {
          backgroundColor: colors.glass.background,
          borderColor: colors.glass.border,
        },
      ]}
    >
      {MODES.map((item) => {
        const active = item.id === mode;
        const Icon = item.icon;
        return (
          <Pressable
            key={item.id}
            accessibilityRole="tab"
            accessibilityLabel={item.label}
            accessibilityState={{ selected: active }}
            onPress={() => onChange(item.id)}
            style={[
              styles.modeButton,
              active && {
                backgroundColor: colors.glass.backgroundElevated,
                borderColor: `${colors.userMessage}42`,
              },
            ]}
          >
            <Icon
              size={17}
              strokeWidth={active ? 2.2 : 1.8}
              color={active ? colors.foreground : colors.mutedForeground}
            />
            {active ? (
              <Text style={[styles.modeLabel, { color: colors.foreground }]}>
                {item.label}
              </Text>
            ) : null}
          </Pressable>
        );
      })}
    </View>
  );
}

function ScreenHeader({
  mode,
  onModeChange,
  onThreads,
  onToolbox,
  colors,
}: {
  mode: Mode;
  onModeChange: (mode: Mode) => void;
  onThreads: () => void;
  onToolbox: () => void;
  colors: ThemeColors;
}) {
  return (
    <View style={styles.header}>
      <Pressable
        accessibilityLabel="Open threads"
        onPress={onThreads}
        style={[
          styles.roundButton,
          {
            borderColor: colors.glass.border,
            backgroundColor: colors.glass.background,
          },
        ]}
      >
        <MessagesSquare
          size={19}
          color={colors.mutedForeground}
          strokeWidth={1.9}
        />
      </Pressable>

      <ModeIsland mode={mode} onChange={onModeChange} colors={colors} />

      <Pressable
        accessibilityLabel="Open toolbox"
        onPress={onToolbox}
        style={[
          styles.roundButton,
          {
            borderColor: colors.glass.border,
            backgroundColor: colors.glass.background,
          },
        ]}
      >
        <Toolbox
          size={19}
          color={colors.mutedForeground}
          strokeWidth={1.9}
        />
      </Pressable>
    </View>
  );
}

function ContextLine({
  mode,
  colors,
}: {
  mode: Mode;
  colors: ThemeColors;
}) {
  const copy = {
    chat: { title: "Mobile navigation system", detail: null },
    code: { title: "Native bottom sheets", detail: "Mitsuro" },
    mako: { title: "Release Captain", detail: "idle" },
  }[mode];

  return (
    <View style={styles.contextLine}>
      <View style={styles.contextCopy}>
        <Text style={[styles.contextTitle, { color: colors.foreground }]}>
          {copy.title}
        </Text>
        {copy.detail ? (
          <Text style={[styles.contextDetail, { color: colors.mutedForeground }]}>
            {copy.detail}
          </Text>
        ) : null}
      </View>
      <Pressable style={styles.moreButton}>
        <MoreHorizontal size={20} color={colors.mutedForeground} />
      </Pressable>
    </View>
  );
}

function ArtifactCard({
  icon: Icon,
  eyebrow,
  title,
  detail,
  accent,
  colors,
}: {
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  detail: string;
  accent: string;
  colors: ThemeColors;
}) {
  return (
    <Pressable
      style={[
        styles.artifactCard,
        {
          backgroundColor: colors.glass.background,
          borderColor: colors.glass.border,
        },
      ]}
    >
      <View style={[styles.artifactIcon, { backgroundColor: `${accent}18` }]}>
        <Icon size={18} color={accent} strokeWidth={1.8} />
      </View>
      <View style={styles.artifactCopy}>
        <Text style={[styles.eyebrow, { color: accent }]}>{eyebrow}</Text>
        <Text style={[styles.artifactTitle, { color: colors.foreground }]}>
          {title}
        </Text>
        <Text
          style={[styles.artifactDetail, { color: colors.mutedForeground }]}
          numberOfLines={2}
        >
          {detail}
        </Text>
      </View>
      <ChevronRight size={17} color={colors.mutedForeground} />
    </Pressable>
  );
}

const CONVERSATION_COPY: Record<
  Mode,
  Array<{
    role: "user" | "assistant";
    text: string;
    artifact?: { type: string; title: string };
    event?: string;
  }>
> = {
  chat: [
    {
      role: "user",
      text: "The three panels are right. I want the thread and toolbox navigation to feel native and stay out of the conversation.",
    },
    {
      role: "assistant",
      text: "Then the conversation should remain the stable canvas. Chat, Code, and Hive change the kind of work, while threads and tools rise temporarily from the bottom.",
      artifact: {
        type: "MARKDOWN REPORT",
        title: "Mobile navigation direction",
      },
    },
  ],
  code: [
    {
      role: "user",
      text: "Match the real SessionDrawer behavior, but present it as a high bottom sheet.",
    },
    {
      role: "assistant",
      text: "I mapped the existing chronological Chat history, project-grouped Code sessions, directory picker, connection state, settings, and new-thread controls.",
      event: "3 files inspected · preview compiling",
    },
  ],
  mako: [
    {
      role: "user",
      text: "You are responsible for release readiness. Watch the project and tell me when something needs attention.",
    },
    {
      role: "assistant",
      text: "Understood. I’ll keep this thread focused on releases, scheduled checks, and deployment readiness. My identity, memory, runs, and schedule stay with this Hive.",
      event: "Next scheduled wake · 4:30 PM",
    },
  ],
};

function ConversationPanel({
  mode,
  colors,
}: {
  mode: Mode;
  colors: ThemeColors;
}) {
  return (
    <ScrollView
      contentContainerStyle={styles.conversationScroll}
      showsVerticalScrollIndicator={false}
    >
      {CONVERSATION_COPY[mode].map((message, index) => {
        const isUser = message.role === "user";
        return (
          <View
            key={`${mode}-${index}`}
            style={[
              styles.messageGroup,
              isUser ? styles.userMessageGroup : styles.assistantMessageGroup,
            ]}
          >
            {!isUser && mode === "mako" ? (
              <View
                style={[
                  styles.messageAvatar,
                  { backgroundColor: `${colors.success}18` },
                ]}
              >
                <HiveIcon size={16} color={colors.success} />
              </View>
            ) : null}
            <View
              style={[
                styles.messageBubble,
                isUser
                  ? {
                      backgroundColor: colors.userMessageBg,
                      borderColor: `${colors.userMessage}20`,
                    }
                  : styles.assistantBubble,
              ]}
            >
              <Text
                style={[
                  styles.messageText,
                  {
                    color: isUser
                      ? colors.foreground
                      : colors.foreground,
                  },
                ]}
              >
                {message.text}
              </Text>

              {message.artifact ? (
                <View
                  style={[
                    styles.inlineArtifact,
                    {
                      backgroundColor: colors.glass.background,
                      borderColor: colors.glass.border,
                    },
                  ]}
                >
                  <FileText size={17} color={colors.userMessage} />
                  <View style={styles.inlineArtifactCopy}>
                    <Text
                      style={[
                        styles.inlineArtifactType,
                        { color: colors.userMessage },
                      ]}
                    >
                      {message.artifact.type}
                    </Text>
                    <Text
                      style={[
                        styles.inlineArtifactTitle,
                        { color: colors.foreground },
                      ]}
                    >
                      {message.artifact.title}
                    </Text>
                  </View>
                  <ChevronRight size={16} color={colors.mutedForeground} />
                </View>
              ) : null}

              {message.event ? (
                <View
                  style={[
                    styles.inlineEvent,
                    {
                      backgroundColor: colors.glass.background,
                      borderColor: colors.glass.border,
                    },
                  ]}
                >
                  {mode === "mako" ? (
                    <CalendarClock size={14} color={colors.success} />
                  ) : (
                    <CircleDot size={14} color={colors.thinking} />
                  )}
                  <Text
                    style={[
                      styles.inlineEventText,
                      { color: colors.mutedForeground },
                    ]}
                  >
                    {message.event}
                  </Text>
                </View>
              ) : null}
            </View>
          </View>
        );
      })}
    </ScrollView>
  );
}

function ChatPanel({ colors }: { colors: ThemeColors }) {
  return <ConversationPanel mode="chat" colors={colors} />;
}

function CodePanel({ colors }: { colors: ThemeColors }) {
  return <ConversationPanel mode="code" colors={colors} />;
}

function MakoPanel({ colors }: { colors: ThemeColors }) {
  return <ConversationPanel mode="mako" colors={colors} />;
}

function Composer({ mode, colors }: { mode: Mode; colors: ThemeColors }) {
  const placeholders = {
    chat: "Ask, create, or explore…",
    code: "Describe what to build…",
    mako: "Set a course for Hive…",
  };

  return (
    <View style={styles.composerWrap}>
      <View
        style={[
          styles.composer,
          {
            backgroundColor: colors.glass.background,
            borderColor: colors.glass.border,
          },
        ]}
      >
        <Pressable style={styles.composerIcon}>
          {mode === "chat" ? (
            <Plus size={19} color={colors.mutedForeground} />
          ) : (
            <Paperclip size={18} color={colors.mutedForeground} />
          )}
        </Pressable>
        <Text style={[styles.placeholder, { color: colors.mutedForeground }]}>
          {placeholders[mode]}
        </Text>
        <Pressable
          style={[
            styles.sendButton,
            { backgroundColor: `${colors.userMessage}1f` },
          ]}
        >
          <Send size={17} color={colors.userMessage} />
        </Pressable>
      </View>
      <Text style={[styles.composerMeta, { color: colors.mutedForeground }]}>
        {mode === "chat"
          ? "GPT 5.6  ·  Work"
          : mode === "code"
            ? "GPT 5.6 Codex  ·  Build"
            : "GPT 5.6  ·  Autonomous"}
      </Text>
    </View>
  );
}

function BottomSheet({
  title,
  subtitle,
  height,
  onClose,
  colors,
  children,
}: {
  title: string;
  subtitle: string;
  height: number;
  onClose: () => void;
  colors: ThemeColors;
  children: ReactNode;
}) {
  const translateY = useRef(new Animated.Value(height)).current;
  const backdropOpacity = useRef(new Animated.Value(0)).current;

  useEffect(() => {
    Animated.parallel([
      Animated.spring(translateY, {
        toValue: 0,
        damping: 24,
        stiffness: 250,
        mass: 0.85,
        useNativeDriver: false,
      }),
      Animated.timing(backdropOpacity, {
        toValue: 1,
        duration: 180,
        useNativeDriver: false,
      }),
    ]).start();
  }, [backdropOpacity, translateY]);

  const close = () => {
    Animated.parallel([
      Animated.timing(translateY, {
        toValue: height,
        duration: 220,
        useNativeDriver: false,
      }),
      Animated.timing(backdropOpacity, {
        toValue: 0,
        duration: 180,
        useNativeDriver: false,
      }),
    ]).start(({ finished }) => {
      if (finished) onClose();
    });
  };

  const drag = useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onMoveShouldSetPanResponder: (_, gesture) =>
          Math.abs(gesture.dy) > Math.abs(gesture.dx),
        onPanResponderMove: (_, gesture) => {
          translateY.setValue(Math.max(0, gesture.dy));
        },
        onPanResponderRelease: (_, gesture) => {
          if (gesture.dy > 76 || gesture.vy > 0.85) {
            close();
            return;
          }
          Animated.spring(translateY, {
            toValue: 0,
            damping: 24,
            stiffness: 280,
            useNativeDriver: false,
          }).start();
        },
      }),
    [height],
  );

  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="box-none">
      <Animated.View
        style={[
          styles.backdrop,
          {
            opacity: backdropOpacity,
          },
        ]}
      >
        <Pressable style={StyleSheet.absoluteFill} onPress={close} />
      </Animated.View>

      <Animated.View
        style={[
          styles.sheet,
          {
            height,
            backgroundColor: colors.background,
            borderColor: colors.glass.border,
            transform: [{ translateY }],
          },
        ]}
      >
        <View style={styles.grabberZone} {...drag.panHandlers}>
          <View
            style={[
              styles.grabber,
              { backgroundColor: colors.mutedForeground },
            ]}
          />
        </View>
        <View style={styles.sheetHeader}>
          <View style={styles.sheetHeading}>
            <Text style={[styles.sheetTitle, { color: colors.foreground }]}>
              {title}
            </Text>
            <Text
              style={[styles.sheetSubtitle, { color: colors.mutedForeground }]}
            >
              {subtitle}
            </Text>
          </View>
        </View>
        <View style={styles.sheetBody}>{children}</View>
      </Animated.View>
    </View>
  );
}

function ThreadSheet({
  mode,
  height,
  colors,
  onClose,
}: {
  mode: Mode;
  height: number;
  colors: ThemeColors;
  onClose: () => void;
}) {
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    new Set(["Mitsuro"]),
  );
  const [folderPickerOpen, setFolderPickerOpen] = useState(false);
  const title =
    mode === "chat" ? "Chats" : mode === "code" ? "Projects" : "Hives";
  const subtitle =
    mode === "chat"
      ? "Most recent first"
      : mode === "code"
        ? "Projects ordered by recent activity"
        : "Persistent personalities and jobs";

  const renderThread = (
    thread: (typeof THREADS.chat)[number],
    icon: ModeIcon,
    accent: string,
  ) => {
    const Icon = icon;
    return (
      <Pressable
        key={thread.title}
        style={[
          styles.threadRow,
          thread.active && {
            backgroundColor: `${accent}10`,
            borderColor: `${accent}24`,
          },
        ]}
      >
        <View
          style={[
            styles.threadIcon,
            {
              backgroundColor: thread.active
                ? `${accent}18`
                : colors.glass.background,
            },
          ]}
        >
          <Icon
            size={17}
            color={thread.active ? accent : colors.mutedForeground}
          />
        </View>
        <View style={styles.threadCopy}>
          <Text
            style={[styles.threadTitle, { color: colors.foreground }]}
            numberOfLines={1}
          >
            {thread.title}
          </Text>
          <Text
            style={[styles.threadDetail, { color: colors.mutedForeground }]}
            numberOfLines={1}
          >
            {thread.detail}
          </Text>
        </View>
        <Text style={[styles.threadTime, { color: colors.mutedForeground }]}>
          {thread.time}
        </Text>
      </Pressable>
    );
  };

  return (
    <BottomSheet
      title={title}
      subtitle={subtitle}
      height={height}
      colors={colors}
      onClose={onClose}
    >
      <ScrollView
        style={styles.threadList}
        contentContainerStyle={styles.threadListContent}
        showsVerticalScrollIndicator={false}
      >
        {mode === "chat"
          ? THREADS.chat.map((thread) =>
              renderThread(thread, MessageCircle, colors.userMessage),
            )
          : null}

        {mode === "code"
          ? CODE_PROJECTS.map((project) => {
              const expanded = expandedProjects.has(project.name);
              return (
                <View key={project.path} style={styles.projectGroup}>
                  <Pressable
                    onPress={() => {
                      setExpandedProjects((current) => {
                        const next = new Set(current);
                        if (next.has(project.name)) next.delete(project.name);
                        else next.add(project.name);
                        return next;
                      });
                    }}
                    style={styles.projectHeader}
                  >
                    {expanded ? (
                      <FolderOpen
                        size={19}
                        color={colors.thinking}
                        strokeWidth={1.7}
                      />
                    ) : (
                      <Folder
                        size={19}
                        color={colors.mutedForeground}
                        strokeWidth={1.7}
                      />
                    )}
                    <View style={styles.projectHeaderCopy}>
                      <Text
                        style={[
                          styles.projectHeaderTitle,
                          {
                            color: expanded
                              ? colors.foreground
                              : colors.mutedForeground,
                          },
                        ]}
                      >
                        {project.name}
                      </Text>
                      <Text
                        style={[
                          styles.projectHeaderPath,
                          { color: colors.mutedForeground },
                        ]}
                        numberOfLines={1}
                      >
                        {project.path}
                      </Text>
                    </View>
                    <Text
                      style={[
                        styles.projectHeaderTime,
                        { color: colors.mutedForeground },
                      ]}
                    >
                      {project.time}
                    </Text>
                    <View
                      style={{
                        transform: [{ rotate: expanded ? "90deg" : "0deg" }],
                      }}
                    >
                      <ChevronRight
                        size={16}
                        color={colors.mutedForeground}
                      />
                    </View>
                  </Pressable>
                  {expanded ? (
                    <View style={styles.projectThreads}>
                      {project.threads.map((thread) =>
                        renderThread(thread, Code2, colors.thinking),
                      )}
                    </View>
                  ) : null}
                </View>
              );
            })
          : null}

        {mode === "mako"
          ? THREADS.mako.map((thread) =>
              renderThread(thread, HiveIcon, colors.success),
            )
          : null}
      </ScrollView>

      {folderPickerOpen ? (
        <View
          style={[
            styles.folderPicker,
            {
              backgroundColor: colors.background,
              borderTopColor: colors.border,
            },
          ]}
        >
          <View style={styles.folderPickerHeader}>
            <View style={styles.folderPickerHeading}>
              <Text
                style={[styles.folderPickerTitle, { color: colors.foreground }]}
              >
                Select Directory
              </Text>
              <Text
                style={[
                  styles.folderPickerPath,
                  { color: colors.mutedForeground },
                ]}
                numberOfLines={1}
              >
                /Users/Jacob/Documents
              </Text>
            </View>
            <Pressable
              onPress={() => setFolderPickerOpen(false)}
              style={styles.footerIconButton}
            >
              <ArrowDown size={20} color={colors.mutedForeground} />
            </Pressable>
          </View>
          {["..", "Krusty", "Sol-Dev", "Krusty-worktrees"].map((directory) => (
            <Pressable key={directory} style={styles.folderPickerRow}>
              <Folder size={18} color={colors.mutedForeground} />
              <Text
                style={[
                  styles.folderPickerRowText,
                  { color: colors.foreground },
                ]}
              >
                {directory}
              </Text>
              <ChevronRight size={16} color={colors.mutedForeground} />
            </Pressable>
          ))}
          <Pressable
            style={[
              styles.selectFolderButton,
              { borderColor: colors.mutedForeground },
            ]}
          >
            <Check size={16} color={colors.mutedForeground} />
            <Text
              style={[
                styles.selectFolderLabel,
                { color: colors.mutedForeground },
              ]}
            >
              Select This Directory
            </Text>
          </Pressable>
        </View>
      ) : null}

      <View style={[styles.sheetFooter, { borderTopColor: colors.border }]}>
        <Pressable
          accessibilityLabel="Settings"
          style={styles.footerIconButton}
        >
          <Settings size={21} color={colors.mutedForeground} />
        </Pressable>
        <View style={styles.connectionStatus}>
          <Wifi size={15} color={colors.success} strokeWidth={2} />
        </View>
        <View style={styles.footerSpacer} />
        <Pressable
          accessibilityLabel="Close threads"
          onPress={onClose}
          style={styles.footerIconButton}
        >
          <ArrowDown size={21} color={colors.mutedForeground} />
        </Pressable>
        <Pressable
          accessibilityLabel={`New ${mode}`}
          style={styles.footerIconButton}
        >
          <SquarePlus size={21} color={colors.mutedForeground} />
        </Pressable>
        {mode === "code" ? (
          <Pressable
            accessibilityLabel="Select project directory"
            onPress={() => setFolderPickerOpen(true)}
            style={styles.footerIconButton}
          >
            <FolderPlus size={21} color={colors.mutedForeground} />
          </Pressable>
        ) : null}
      </View>
    </BottomSheet>
  );
}

function ToolboxSheet({
  mode,
  height,
  colors,
  onClose,
}: {
  mode: Mode;
  height: number;
  colors: ThemeColors;
  onClose: () => void;
}) {
  const tabs = TOOL_TABS[mode];
  const [activeTab, setActiveTab] = useState(tabs[0].id);

  useEffect(() => {
    setActiveTab(tabs[0].id);
  }, [mode, tabs]);

  return (
    <BottomSheet
      title={mode === "chat" ? "Work" : mode === "code" ? "Toolbox" : "Hive"}
      subtitle={
        mode === "chat"
          ? "Artifacts, plugins, and reusable work"
          : mode === "code"
            ? "Project tools for this code thread"
            : "Schedules, runs, and durable memory"
      }
      height={height}
      colors={colors}
      onClose={onClose}
    >
      <ScrollView
        style={styles.toolContent}
        contentContainerStyle={styles.toolContentInner}
        showsVerticalScrollIndicator={false}
      >
        <ToolPreview mode={mode} activeTab={activeTab} colors={colors} />
      </ScrollView>

      <View style={[styles.toolDock, { borderTopColor: colors.border }]}>
        <View
          style={[
            styles.toolRail,
            {
              backgroundColor: colors.glass.background,
              borderColor: colors.glass.border,
            },
          ]}
        >
          {tabs.map((tab) => {
            const Icon = tab.icon;
            const active = activeTab === tab.id;
            return (
              <Pressable
                key={tab.id}
                onPress={() => setActiveTab(tab.id)}
                style={[
                  styles.toolTab,
                  active && {
                    backgroundColor: colors.glass.backgroundElevated,
                  },
                ]}
              >
                <Icon
                  size={17}
                  color={active ? colors.foreground : colors.mutedForeground}
                />
                <Text
                  style={[
                    styles.toolTabLabel,
                    {
                      color: active
                        ? colors.foreground
                        : colors.mutedForeground,
                    },
                  ]}
                >
                  {tab.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
      </View>
    </BottomSheet>
  );
}

function ToolPreview({
  mode,
  activeTab,
  colors,
}: {
  mode: Mode;
  activeTab: string;
  colors: ThemeColors;
}) {
  if (mode === "chat") {
    if (activeTab === "plugins") {
      return (
        <>
          <Text style={[styles.toolHeading, { color: colors.foreground }]}>
            Enabled for this space
          </Text>
          <ToolRow
            icon={Globe2}
            title="Browser"
            detail="Research and inspect the web"
            badge="Connected"
            colors={colors}
          />
          <ToolRow
            icon={Blocks}
            title="GitHub"
            detail="Repositories, issues, and pull requests"
            badge="Connected"
            colors={colors}
          />
          <ToolRow
            icon={Plus}
            title="Add plugin"
            detail="Extend what Chat can access and create"
            colors={colors}
          />
        </>
      );
    }
    if (activeTab === "history") {
      return (
        <>
          <Text style={[styles.toolHeading, { color: colors.foreground }]}>
            Workspace library
          </Text>
          <ToolRow
            icon={History}
            title="Navigation decisions"
            detail="8 saved decisions · updated today"
            colors={colors}
          />
          <ToolRow
            icon={FileText}
            title="Research reports"
            detail="12 Markdown reports"
            colors={colors}
          />
          <ToolRow
            icon={Globe2}
            title="Interactive artifacts"
            detail="4 HTML experiences"
            colors={colors}
          />
        </>
      );
    }
    return (
      <>
        <View style={styles.toolHeadingRow}>
          <Text style={[styles.toolHeading, { color: colors.foreground }]}>
            Artifacts
          </Text>
          <Text style={[styles.toolLink, { color: colors.userMessage }]}>
            Create
          </Text>
        </View>
        <ArtifactTile
          type="MARKDOWN"
          title="Mobile navigation brief"
          detail="Deep report · edited 4m ago"
          icon={FileText}
          accent={colors.userMessage}
          colors={colors}
        />
        <ArtifactTile
          type="HTML"
          title="Navigation prototype"
          detail="Interactive · open now"
          icon={Globe2}
          accent={colors.thinking}
          colors={colors}
        />
      </>
    );
  }

  if (mode === "code") {
    if (activeTab === "terminal") {
      return (
        <View
          style={[
            styles.terminal,
            { borderColor: colors.glass.border, backgroundColor: "#071018" },
          ]}
        >
          <Text style={styles.terminalMeta}>krusty — zsh — 80×24</Text>
          <Text style={styles.terminalText}>
            <Text style={{ color: colors.success }}>➜ </Text>
            <Text style={{ color: colors.userMessage }}>Mitsuro </Text>
            git status --short{"\n"}
            <Text style={{ color: colors.warning }}> M </Text>
            apps/mobile/components/chat/ChatBar.tsx{"\n"}
            <Text style={{ color: colors.warning }}>?? </Text>
            apps/mobile/app/navigation-preview.tsx{"\n"}
            <Text style={{ color: colors.success }}>➜ </Text>
            <Text style={{ color: colors.userMessage }}>Mitsuro </Text>
            <Text style={{ color: colors.foreground }}>▋</Text>
          </Text>
        </View>
      );
    }
    if (activeTab === "changes") {
      return (
        <>
          <Text style={[styles.toolHeading, { color: colors.foreground }]}>
            Working changes
          </Text>
          <ToolRow
            icon={FileCode2}
            title="navigation-preview.tsx"
            detail="+ isolated visual prototype"
            badge="New"
            colors={colors}
          />
          <ToolRow
            icon={FileCode2}
            title="ChatBar.tsx"
            detail="Existing local changes preserved"
            badge="Modified"
            colors={colors}
          />
        </>
      );
    }
    return (
      <View
        style={[
          styles.browserPreview,
          { borderColor: colors.glass.border, backgroundColor: "#071018" },
        ]}
      >
        <View style={[styles.browserBar, { borderBottomColor: colors.border }]}>
          <View style={styles.browserDots}>
            <View style={[styles.browserDot, { backgroundColor: "#ff6b6b" }]} />
            <View style={[styles.browserDot, { backgroundColor: "#ffd166" }]} />
            <View style={[styles.browserDot, { backgroundColor: "#63d69a" }]} />
          </View>
          <View
            style={[
              styles.addressBar,
              { backgroundColor: colors.glass.background },
            ]}
          >
            <Text
              style={[styles.addressText, { color: colors.mutedForeground }]}
              numberOfLines={1}
            >
              localhost:5173/navigation-preview
            </Text>
          </View>
        </View>
        <View style={styles.browserCanvas}>
          <Globe2 size={28} color={colors.userMessage} />
          <Text style={[styles.browserTitle, { color: colors.foreground }]}>
            Live project preview
          </Text>
          <Text style={[styles.browserDetail, { color: colors.mutedForeground }]}>
            Browser and terminal remain project tools, not global navigation.
          </Text>
        </View>
      </View>
    );
  }

  if (activeTab === "runs") {
    return (
      <>
        <Text style={[styles.toolHeading, { color: colors.foreground }]}>
          Runs
        </Text>
        <ToolRow
          icon={Workflow}
          title="Navigation research"
          detail="Running · 6m · Mitsuro"
          badge="Active"
          colors={colors}
        />
        <ToolRow
          icon={Check}
          title="Dependency review"
          detail="Completed · 42m ago"
          badge="Done"
          colors={colors}
        />
      </>
    );
  }
  if (activeTab === "memory") {
    return (
      <>
        <Text style={[styles.toolHeading, { color: colors.foreground }]}>
          Durable memory
        </Text>
        <ToolRow
          icon={MemoryStick}
          title="Mitsuro project knowledge"
          detail="34 facts · 12 procedures"
          colors={colors}
        />
        <ToolRow
          icon={Archive}
          title="Conversation episodes"
          detail="Recent work available for recall"
          colors={colors}
        />
      </>
    );
  }
  return (
    <>
      <View style={styles.toolHeadingRow}>
        <Text style={[styles.toolHeading, { color: colors.foreground }]}>
          Schedule
        </Text>
        <Text style={[styles.toolLink, { color: colors.userMessage }]}>
          Add
        </Text>
      </View>
      <ToolRow
        icon={CalendarClock}
        title="Review implementation work"
        detail="Today at 4:30 PM · enabled"
        badge="Next"
        colors={colors}
      />
      <ToolRow
        icon={Clock3}
        title="Nightly dependency review"
        detail="Every day at 11:00 PM"
        colors={colors}
      />
    </>
  );
}

function ToolRow({
  icon: Icon,
  title,
  detail,
  badge,
  colors,
}: {
  icon: LucideIcon;
  title: string;
  detail: string;
  badge?: string;
  colors: ThemeColors;
}) {
  return (
    <Pressable
      style={[
        styles.toolRow,
        {
          backgroundColor: colors.glass.background,
          borderColor: colors.glass.border,
        },
      ]}
    >
      <View
        style={[
          styles.toolRowIcon,
          { backgroundColor: `${colors.userMessage}13` },
        ]}
      >
        <Icon size={18} color={colors.userMessage} />
      </View>
      <View style={styles.toolRowCopy}>
        <Text style={[styles.toolRowTitle, { color: colors.foreground }]}>
          {title}
        </Text>
        <Text style={[styles.toolRowDetail, { color: colors.mutedForeground }]}>
          {detail}
        </Text>
      </View>
      {badge ? (
        <Text style={[styles.toolBadge, { color: colors.success }]}>{badge}</Text>
      ) : (
        <ChevronRight size={17} color={colors.mutedForeground} />
      )}
    </Pressable>
  );
}

function ArtifactTile({
  type,
  title,
  detail,
  icon: Icon,
  accent,
  colors,
}: {
  type: string;
  title: string;
  detail: string;
  icon: LucideIcon;
  accent: string;
  colors: ThemeColors;
}) {
  return (
    <Pressable
      style={[
        styles.artifactTile,
        {
          borderColor: colors.glass.border,
          backgroundColor: colors.glass.background,
        },
      ]}
    >
      <View style={[styles.artifactTilePreview, { backgroundColor: `${accent}10` }]}>
        <Icon size={26} color={accent} strokeWidth={1.6} />
      </View>
      <View style={styles.artifactTileCopy}>
        <Text style={[styles.eyebrow, { color: accent }]}>{type}</Text>
        <Text style={[styles.artifactTileTitle, { color: colors.foreground }]}>
          {title}
        </Text>
        <Text
          style={[styles.artifactTileDetail, { color: colors.mutedForeground }]}
        >
          {detail}
        </Text>
      </View>
    </Pressable>
  );
}

export default function NavigationPreview() {
  const { theme } = useThemeContext();
  const colors = theme.colors;
  const insets = useSafeAreaInsets();
  const { height: windowHeight } = useWindowDimensions();
  const [mode, setMode] = useState<Mode>("chat");
  const [sheet, setSheet] = useState<SheetKind>(null);
  const modeRef = useRef(mode);
  modeRef.current = mode;

  const changeMode = (next: Mode) => {
    setMode(next);
  };

  const swipeResponder = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_, gesture) =>
          Math.abs(gesture.dx) > 24 &&
          Math.abs(gesture.dx) > Math.abs(gesture.dy) * 1.4,
        onPanResponderRelease: (_, gesture) => {
          const current = MODE_INDEX[modeRef.current];
          if (gesture.dx < -58 && current < MODES.length - 1) {
            setMode(MODES[current + 1].id);
          } else if (gesture.dx > 58 && current > 0) {
            setMode(MODES[current - 1].id);
          }
        },
      }),
    [],
  );

  const sheetHeight = Math.min(
    windowHeight * (sheet === "threads" ? 0.91 : 0.72),
    820,
  );

  return (
    <View style={[styles.screen, { backgroundColor: colors.background }]}>
      <View
        style={[
          styles.safeContent,
          {
            paddingTop: Math.max(insets.top, 16) + 4,
            paddingBottom: Math.max(insets.bottom, 10),
          },
        ]}
      >
        <ScreenHeader
          mode={mode}
          onModeChange={changeMode}
          onThreads={() => setSheet("threads")}
          onToolbox={() => setSheet("toolbox")}
          colors={colors}
        />
        <ContextLine mode={mode} colors={colors} />

        <View style={styles.panelHost} {...swipeResponder.panHandlers}>
          {mode === "chat" ? (
            <ChatPanel colors={colors} />
          ) : mode === "code" ? (
            <CodePanel colors={colors} />
          ) : (
            <MakoPanel colors={colors} />
          )}
        </View>

        <Composer mode={mode} colors={colors} />
      </View>

      {sheet === "threads" ? (
        <ThreadSheet
          mode={mode}
          height={sheetHeight}
          colors={colors}
          onClose={() => setSheet(null)}
        />
      ) : null}

      {sheet === "toolbox" ? (
        <ToolboxSheet
          mode={mode}
          height={sheetHeight}
          colors={colors}
          onClose={() => setSheet(null)}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
  },
  safeContent: {
    flex: 1,
    paddingHorizontal: 14,
  },
  header: {
    height: 48,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  roundButton: {
    width: 40,
    height: 40,
    borderRadius: 20,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  modeIsland: {
    flexDirection: "row",
    alignItems: "center",
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 3,
    gap: 2,
  },
  modeButton: {
    height: 34,
    minWidth: 38,
    paddingHorizontal: 10,
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "transparent",
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
  },
  modeLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  contextLine: {
    minHeight: 68,
    paddingTop: 13,
    paddingHorizontal: 4,
    flexDirection: "row",
    alignItems: "flex-start",
  },
  contextCopy: {
    flex: 1,
  },
  contextTitle: {
    fontSize: 20,
    fontWeight: "600",
    letterSpacing: -0.35,
  },
  contextDetail: {
    fontSize: 12,
    fontWeight: "500",
    marginTop: 3,
  },
  moreButton: {
    width: 34,
    height: 34,
    alignItems: "center",
    justifyContent: "center",
  },
  panelHost: {
    flex: 1,
  },
  panelScroll: {
    paddingTop: 16,
    paddingBottom: 26,
  },
  conversationScroll: {
    paddingTop: 18,
    paddingHorizontal: 2,
    paddingBottom: 34,
    gap: 22,
  },
  messageGroup: {
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 9,
  },
  userMessageGroup: {
    alignSelf: "flex-end",
    width: "86%",
    maxWidth: "86%",
    justifyContent: "flex-end",
  },
  assistantMessageGroup: {
    alignSelf: "stretch",
  },
  messageAvatar: {
    width: 30,
    height: 30,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 1,
  },
  messageBubble: {
    maxWidth: "100%",
    flexShrink: 1,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 11,
  },
  assistantBubble: {
    flex: 1,
    borderWidth: 0,
    paddingHorizontal: 3,
    paddingVertical: 4,
  },
  messageText: {
    fontSize: 14,
    lineHeight: 21,
  },
  inlineArtifact: {
    minHeight: 60,
    marginTop: 13,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 11,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  inlineArtifactCopy: {
    flex: 1,
  },
  inlineArtifactType: {
    fontSize: 8,
    fontWeight: "800",
    letterSpacing: 0.7,
  },
  inlineArtifactTitle: {
    marginTop: 3,
    fontSize: 12,
    fontWeight: "600",
  },
  inlineEvent: {
    alignSelf: "flex-start",
    minHeight: 34,
    marginTop: 13,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
  },
  inlineEventText: {
    fontSize: 10,
    fontWeight: "500",
  },
  hero: {
    alignItems: "center",
    paddingHorizontal: 22,
    paddingTop: 18,
    paddingBottom: 34,
  },
  heroMark: {
    width: 50,
    height: 50,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 16,
  },
  heroTitle: {
    fontSize: 23,
    fontWeight: "600",
    letterSpacing: -0.5,
    textAlign: "center",
  },
  heroDetail: {
    maxWidth: 310,
    marginTop: 9,
    fontSize: 13,
    lineHeight: 19,
    textAlign: "center",
  },
  sectionHeading: {
    marginBottom: 10,
    paddingHorizontal: 2,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  sectionTitle: {
    fontSize: 15,
    fontWeight: "700",
  },
  sectionAction: {
    fontSize: 12,
    fontWeight: "600",
  },
  artifactCard: {
    minHeight: 88,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 13,
    marginBottom: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  artifactIcon: {
    width: 42,
    height: 42,
    borderRadius: 13,
    alignItems: "center",
    justifyContent: "center",
  },
  artifactCopy: {
    flex: 1,
    minWidth: 0,
  },
  eyebrow: {
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  artifactTitle: {
    marginTop: 3,
    fontSize: 14,
    fontWeight: "600",
  },
  artifactDetail: {
    marginTop: 3,
    fontSize: 11,
    lineHeight: 15,
  },
  projectCard: {
    borderRadius: 19,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
    marginBottom: 26,
  },
  projectTop: {
    minHeight: 82,
    padding: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  projectIcon: {
    width: 44,
    height: 44,
    borderRadius: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  projectCopy: {
    flex: 1,
    minWidth: 0,
  },
  projectName: {
    fontSize: 16,
    fontWeight: "700",
  },
  projectPath: {
    fontSize: 11,
    marginTop: 4,
  },
  branchRow: {
    minHeight: 42,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  branchText: {
    flex: 1,
    fontSize: 11,
  },
  cleanText: {
    fontSize: 10,
    fontWeight: "700",
    textTransform: "uppercase",
  },
  taskCard: {
    borderRadius: 19,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 16,
  },
  taskHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
  },
  taskTitle: {
    flex: 1,
    fontSize: 15,
    fontWeight: "700",
  },
  taskBody: {
    marginTop: 12,
    fontSize: 12,
    lineHeight: 18,
  },
  progressRow: {
    marginTop: 18,
    flexDirection: "row",
    gap: 20,
  },
  progressStep: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  progressDot: {
    width: 17,
    height: 17,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
  },
  progressLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  makoStatus: {
    borderRadius: 19,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 16,
    marginBottom: 12,
  },
  makoStatusTop: {
    flexDirection: "row",
    alignItems: "center",
  },
  liveDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    marginRight: 9,
  },
  makoStatusTitle: {
    flex: 1,
    fontSize: 15,
    fontWeight: "700",
  },
  liveLabel: {
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  makoStatusDetail: {
    marginTop: 9,
    fontSize: 12,
    lineHeight: 17,
  },
  metrics: {
    flexDirection: "row",
    gap: 10,
    marginBottom: 26,
  },
  metricCard: {
    flex: 1,
    minHeight: 112,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 14,
  },
  metricValue: {
    marginTop: 13,
    fontSize: 22,
    fontWeight: "700",
  },
  metricLabel: {
    marginTop: 2,
    fontSize: 11,
  },
  scheduleCard: {
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 13,
    minHeight: 78,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  scheduleIcon: {
    width: 42,
    height: 42,
    borderRadius: 13,
    alignItems: "center",
    justifyContent: "center",
  },
  scheduleCopy: {
    flex: 1,
  },
  scheduleTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  scheduleDetail: {
    fontSize: 11,
    marginTop: 4,
  },
  composerWrap: {
    paddingTop: 8,
  },
  composer: {
    minHeight: 50,
    borderRadius: 19,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 6,
    flexDirection: "row",
    alignItems: "center",
  },
  composerIcon: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  placeholder: {
    flex: 1,
    fontSize: 13,
  },
  sendButton: {
    width: 36,
    height: 36,
    borderRadius: 13,
    alignItems: "center",
    justifyContent: "center",
  },
  composerMeta: {
    textAlign: "right",
    paddingRight: 5,
    marginTop: 5,
    fontSize: 9,
  },
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0,0,0,0.56)",
    zIndex: 900,
  },
  sheet: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 901,
    borderTopLeftRadius: 28,
    borderTopRightRadius: 28,
    borderWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: 0,
    overflow: "hidden",
  },
  grabberZone: {
    height: 28,
    alignItems: "center",
    justifyContent: "center",
  },
  grabber: {
    width: 42,
    height: 5,
    borderRadius: 3,
    opacity: 0.42,
  },
  sheetHeader: {
    minHeight: 58,
    paddingHorizontal: 18,
    paddingBottom: 12,
    flexDirection: "row",
    alignItems: "center",
  },
  sheetHeading: {
    flex: 1,
  },
  sheetTitle: {
    fontSize: 20,
    fontWeight: "700",
    letterSpacing: -0.35,
  },
  sheetSubtitle: {
    marginTop: 3,
    fontSize: 11,
  },
  sheetBody: {
    flex: 1,
  },
  sheetModeRail: {
    alignSelf: "center",
    flexDirection: "row",
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 3,
    gap: 2,
  },
  sheetModeButton: {
    height: 34,
    paddingHorizontal: 13,
    borderRadius: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  sheetModeLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  threadList: {
    flex: 1,
    marginTop: 14,
  },
  threadListContent: {
    paddingHorizontal: 14,
    paddingBottom: 14,
  },
  projectGroup: {
    marginBottom: 8,
  },
  projectHeader: {
    minHeight: 64,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  projectHeaderCopy: {
    flex: 1,
    minWidth: 0,
  },
  projectHeaderTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  projectHeaderPath: {
    marginTop: 3,
    fontSize: 9,
  },
  projectHeaderTime: {
    fontSize: 9,
  },
  projectThreads: {
    paddingLeft: 12,
  },
  listEyebrow: {
    marginHorizontal: 6,
    marginBottom: 8,
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  threadRow: {
    minHeight: 68,
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "transparent",
    paddingHorizontal: 11,
    flexDirection: "row",
    alignItems: "center",
    gap: 11,
  },
  threadIcon: {
    width: 38,
    height: 38,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  threadCopy: {
    flex: 1,
    minWidth: 0,
  },
  threadTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  threadDetail: {
    marginTop: 4,
    fontSize: 10,
  },
  threadTime: {
    fontSize: 10,
  },
  sheetFooter: {
    minHeight: 68,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 18,
    paddingVertical: 11,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
  },
  footerIconButton: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  connectionStatus: {
    width: 28,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  footerSpacer: {
    flex: 1,
  },
  folderPicker: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 68,
    zIndex: 8,
    minHeight: 390,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 17,
    paddingTop: 14,
  },
  folderPickerHeader: {
    flexDirection: "row",
    alignItems: "flex-start",
    marginBottom: 12,
  },
  folderPickerHeading: {
    flex: 1,
  },
  folderPickerTitle: {
    fontSize: 16,
    fontWeight: "700",
  },
  folderPickerPath: {
    marginTop: 3,
    fontSize: 10,
  },
  folderPickerRow: {
    minHeight: 49,
    flexDirection: "row",
    alignItems: "center",
    gap: 11,
    paddingHorizontal: 4,
  },
  folderPickerRowText: {
    flex: 1,
    fontSize: 14,
  },
  selectFolderButton: {
    minHeight: 43,
    marginTop: 9,
    borderRadius: 14,
    borderWidth: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  selectFolderLabel: {
    fontSize: 13,
    fontWeight: "600",
  },
  newThreadButton: {
    flex: 1,
    height: 45,
    borderRadius: 15,
    alignItems: "center",
    justifyContent: "center",
    flexDirection: "row",
    gap: 8,
  },
  newThreadLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  searchButton: {
    width: 45,
    height: 45,
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  toolRail: {
    alignSelf: "center",
    flexDirection: "row",
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 3,
    gap: 2,
  },
  toolDock: {
    minHeight: 58,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  toolTab: {
    height: 36,
    paddingHorizontal: 12,
    borderRadius: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  toolTabLabel: {
    fontSize: 10,
    fontWeight: "600",
  },
  toolContent: {
    flex: 1,
    marginTop: 14,
  },
  toolContentInner: {
    paddingHorizontal: 14,
    paddingBottom: 24,
  },
  toolHeadingRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  toolHeading: {
    fontSize: 15,
    fontWeight: "700",
    marginBottom: 11,
  },
  toolLink: {
    fontSize: 11,
    fontWeight: "700",
    marginBottom: 11,
  },
  toolRow: {
    minHeight: 67,
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 11,
    marginBottom: 9,
    flexDirection: "row",
    alignItems: "center",
    gap: 11,
  },
  toolRowIcon: {
    width: 38,
    height: 38,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  toolRowCopy: {
    flex: 1,
    minWidth: 0,
  },
  toolRowTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  toolRowDetail: {
    fontSize: 10,
    marginTop: 4,
  },
  toolBadge: {
    fontSize: 9,
    fontWeight: "800",
    textTransform: "uppercase",
  },
  artifactTile: {
    minHeight: 110,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    marginBottom: 10,
    padding: 10,
    flexDirection: "row",
    gap: 12,
  },
  artifactTilePreview: {
    width: 88,
    borderRadius: 13,
    alignItems: "center",
    justifyContent: "center",
  },
  artifactTileCopy: {
    flex: 1,
    justifyContent: "center",
  },
  artifactTileTitle: {
    fontSize: 14,
    fontWeight: "700",
    marginTop: 5,
  },
  artifactTileDetail: {
    fontSize: 10,
    marginTop: 5,
  },
  terminal: {
    minHeight: 250,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 14,
  },
  terminalMeta: {
    color: "#607080",
    fontSize: 9,
    marginBottom: 16,
    fontFamily: "monospace",
  },
  terminalText: {
    color: "#b5c1cd",
    fontSize: 11,
    lineHeight: 19,
    fontFamily: "monospace",
  },
  browserPreview: {
    minHeight: 270,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  browserBar: {
    height: 47,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 11,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  browserDots: {
    flexDirection: "row",
    gap: 5,
  },
  browserDot: {
    width: 7,
    height: 7,
    borderRadius: 4,
  },
  addressBar: {
    flex: 1,
    height: 27,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 9,
  },
  addressText: {
    fontSize: 9,
  },
  browserCanvas: {
    flex: 1,
    minHeight: 220,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 35,
  },
  browserTitle: {
    marginTop: 13,
    fontSize: 15,
    fontWeight: "700",
  },
  browserDetail: {
    marginTop: 7,
    fontSize: 11,
    lineHeight: 16,
    textAlign: "center",
  },
});
