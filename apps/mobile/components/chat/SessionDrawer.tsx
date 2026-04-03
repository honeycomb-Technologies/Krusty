import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import {
  View,
  Text,
  Pressable,
  ScrollView,
  StyleSheet,
  Dimensions,
} from 'react-native';
import { BlurView } from '../../platform/blur';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withTiming,
  interpolate,
  runOnJS,
  Easing,
} from 'react-native-reanimated';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import { ArrowLeft, Settings, SquarePlus, FolderPlus, Folder, FolderOpen, ChevronRight, ChevronDown, ChevronLeft, Check, Wifi, WifiOff } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { SegmentControl } from '../ui/SegmentControl';
import type { SessionResponse } from '@krusty/api';

const DRAWER_WIDTH = Dimensions.get('window').width * 0.82;
const SPRING = { damping: 22, stiffness: 300, mass: 0.8 };
const PICKER_HEIGHT = Dimensions.get('window').height * 0.52;

interface SessionDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onNewSession: () => void;
  onNewSessionWithDir: (path: string) => void;
  onDeleteSession: (id: string) => void;
  onOpenSettings?: () => void;
  activeTab: number;
  onTabChange: (index: number) => void;
}

interface DirEntry { name: string; path: string }
interface DirCache { current: string; parent: string | null; directories: DirEntry[] }

function dirDisplayName(path: string): string {
  if (path === 'Neutral') return 'General';
  const parts = path.split('/').filter(Boolean);
  return parts.slice(-2).join('/');
}

function formatTime(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function typeLabel(type: SessionResponse['session_type']): string {
  switch (type) {
    case 'chat': return 'Chat';
    case 'mako': return 'Mako';
    default: return 'Code';
  }
}

export function SessionDrawer({
  isOpen, onClose, sessions, activeSessionId,
  onSelectSession, onNewSession, onNewSessionWithDir, onDeleteSession,
  onOpenSettings, activeTab, onTabChange,
}: SessionDrawerProps) {
  const { theme } = useThemeContext();
  const { client, status } = useConnection();
  const translateX = useSharedValue(-DRAWER_WIDTH);
  const backdropOpacity = useSharedValue(0);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

  // Directory picker — animated slide-up
  const pickerProgress = useSharedValue(0); // 0 = hidden, 1 = fully visible
  const [pickerVisible, setPickerVisible] = useState(false);
  const [pickerPath, setPickerPath] = useState('');
  const [pickerParent, setPickerParent] = useState<string | null>(null);
  const [pickerDirs, setPickerDirs] = useState<DirEntry[]>([]);
  const dirCache = useRef<Map<string, DirCache>>(new Map());
  const [pickerReady, setPickerReady] = useState(false); // data loaded at least once

  // Pre-fetch home directory when drawer opens on Code/Mako tab
  useEffect(() => {
    if (isOpen && client && activeTab !== 0 && !pickerReady) {
      client.browseDirectories().then(result => {
        const entry: DirCache = { current: result.current, parent: result.parent, directories: result.directories };
        dirCache.current.set('', entry);
        dirCache.current.set(result.current, entry);
        setPickerPath(result.current);
        setPickerParent(result.parent);
        setPickerDirs(result.directories);
        setPickerReady(true);
      }).catch(() => {});
    }
  }, [isOpen, client, activeTab, pickerReady]);

  // Reset picker when drawer closes
  useEffect(() => {
    if (!isOpen) {
      pickerProgress.value = withTiming(0, { duration: 150 });
      setTimeout(() => setPickerVisible(false), 160);
    }
  }, [isOpen]);

  const navigatePicker = useCallback(async (path: string) => {
    if (!client) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const cached = dirCache.current.get(path);
    if (cached) {
      setPickerPath(cached.current);
      setPickerParent(cached.parent);
      setPickerDirs(cached.directories);
      return;
    }
    try {
      const result = await client.browseDirectories(path || undefined);
      const entry: DirCache = { current: result.current, parent: result.parent, directories: result.directories };
      dirCache.current.set(path, entry);
      dirCache.current.set(result.current, entry);
      setPickerPath(result.current);
      setPickerParent(result.parent);
      setPickerDirs(result.directories);
    } catch { /* keep current view */ }
  }, [client]);

  const showPicker = useCallback(() => {
    setPickerVisible(true);
    pickerProgress.value = withSpring(1, { damping: 20, stiffness: 250, mass: 0.8 });
    // If not pre-loaded, fetch now
    if (!pickerReady && client) {
      client.browseDirectories().then(result => {
        const entry: DirCache = { current: result.current, parent: result.parent, directories: result.directories };
        dirCache.current.set('', entry);
        setPickerPath(result.current);
        setPickerParent(result.parent);
        setPickerDirs(result.directories);
        setPickerReady(true);
      }).catch(() => {});
    }
  }, [pickerReady, client]);

  const hidePicker = useCallback(() => {
    pickerProgress.value = withTiming(0, { duration: 200, easing: Easing.out(Easing.cubic) });
    setTimeout(() => setPickerVisible(false), 210);
  }, []);

  const pickerAnimStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: interpolate(pickerProgress.value, [0, 1], [PICKER_HEIGHT, 0]) }],
    opacity: pickerProgress.value,
  }));

  // Drawer open/close animation
  useEffect(() => {
    translateX.value = withSpring(isOpen ? 0 : -DRAWER_WIDTH, SPRING);
    backdropOpacity.value = withSpring(isOpen ? 1 : 0, SPRING);
  }, [isOpen]);

  const drawerStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: translateX.value }],
  }));
  const backdropStyle = useAnimatedStyle(() => ({
    opacity: backdropOpacity.value,
    pointerEvents: backdropOpacity.value > 0.1 ? 'auto' as const : 'none' as const,
  }));

  const swipeGesture = Gesture.Pan()
    .onUpdate(e => { if (e.translationX < 0) translateX.value = e.translationX; })
    .onEnd(e => {
      if (e.translationX < -80 || e.velocityX < -500) {
        translateX.value = withSpring(-DRAWER_WIDTH, SPRING);
        backdropOpacity.value = withSpring(0, SPRING);
        runOnJS(onClose)();
      } else {
        translateX.value = withSpring(0, SPRING);
      }
    });

  const t = theme.colors;
  const g = theme.colors.glass;

  const chatSessions = useMemo(() =>
    sessions.filter(s => s.session_type === 'chat')
      .sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()),
    [sessions]
  );

  const groupSessionsByDirectory = (type: SessionResponse['session_type']) => {
    const groups = new Map<string, SessionResponse[]>();
    for (const s of sessions) {
      if (s.session_type !== type) continue;
      const dir = s.project_dir ?? s.working_dir ?? 'Neutral';
      const list = groups.get(dir) ?? [];
      list.push(s);
      groups.set(dir, list);
    }
    for (const [, list] of groups) {
      list.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
    }
    return Array.from(groups.entries()).sort(([a], [b]) => {
      if (a === 'Neutral') return -1;
      if (b === 'Neutral') return 1;
      return a.localeCompare(b);
    });
  };

  const codeDirGroups = useMemo(() => groupSessionsByDirectory('code'), [sessions]);
  const makoDirGroups = useMemo(() => groupSessionsByDirectory('mako'), [sessions]);

  const toggleDir = (dir: string) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setExpandedDirs(prev => {
      const next = new Set(prev);
      next.has(dir) ? next.delete(dir) : next.add(dir);
      return next;
    });
  };

  const renderSessionItem = (item: SessionResponse) => {
    const isActive = item.id === activeSessionId;
    return (
      <Pressable
        key={item.id}
        onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); onSelectSession(item); }}
        onLongPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy); onDeleteSession(item.id); }}
        style={[styles.sessionItem, isActive && { backgroundColor: t.userMessage + '12' }]}
      >
        <Text style={[styles.sessionTitle, { color: isActive ? t.userMessage : t.foreground }]} numberOfLines={2}>
          {item.title || 'Untitled'}
        </Text>
        <View style={styles.sessionMeta}>
          <Text style={[styles.sessionTime, { color: t.mutedForeground }]}>{formatTime(item.updated_at)}</Text>
          {item.model && (
            <Text style={[styles.sessionModel, { color: t.mutedForeground }]} numberOfLines={1}>
              {item.model.length > 18 ? item.model.slice(0, 18) + '…' : item.model}
            </Text>
          )}
        </View>
      </Pressable>
    );
  };

  const renderDirAccordion = (groups: [string, SessionResponse[]][], folderColor: string) =>
    groups.length === 0
      ? <Text style={[styles.emptyText, { color: t.mutedForeground }]}>No sessions</Text>
      : groups.map(([dir, dirSessions]) => {
          const expanded = expandedDirs.has(dir);
          return (
            <View key={dir}>
              <Pressable onPress={() => toggleDir(dir)} style={styles.dirHeader}>
                {expanded
                  ? <FolderOpen size={18} color={folderColor} strokeWidth={1.6} />
                  : <Folder size={18} color={t.mutedForeground} strokeWidth={1.6} />}
                <Text style={[styles.dirName, { color: expanded ? t.foreground : t.mutedForeground }]} numberOfLines={1}>
                  {dirDisplayName(dir)}
                </Text>
                <Text style={[styles.dirCount, { color: t.mutedForeground }]}>({dirSessions.length})</Text>
                {expanded ? <ChevronDown size={16} color={t.mutedForeground} /> : <ChevronRight size={16} color={t.mutedForeground} />}
              </Pressable>
              {expanded && <View style={styles.dirSessions}>{dirSessions.map(renderSessionItem)}</View>}
            </View>
          );
        });

  return (
    <>
      <Animated.View style={[styles.backdrop, backdropStyle]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
      </Animated.View>

      <GestureDetector gesture={swipeGesture}>
        <Animated.View style={[styles.drawer, { width: DRAWER_WIDTH }, drawerStyle]}>
          <BlurView
            intensity={40}
            tint={theme.scheme === 'dark' ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight'}
            style={StyleSheet.absoluteFill}
          />
          <View style={[StyleSheet.absoluteFill, {
            backgroundColor: theme.scheme === 'dark' ? 'rgba(11,17,25,0.88)' : 'rgba(255,255,255,0.88)',
          }]} />

          <View style={styles.content}>
            <View style={styles.segmentWrap}>
              <SegmentControl segments={['Chat', 'Code', 'Mako']} selected={activeTab} onSelect={onTabChange} />
            </View>

            <ScrollView style={styles.listArea} showsVerticalScrollIndicator={false}>
              {activeTab === 0 && (
                chatSessions.length === 0
                  ? <Text style={[styles.emptyText, { color: t.mutedForeground }]}>No chat sessions</Text>
                  : chatSessions.map(renderSessionItem)
              )}
              {activeTab === 1 && renderDirAccordion(codeDirGroups, t.thinking)}
              {activeTab === 2 && renderDirAccordion(makoDirGroups, t.userMessage)}
            </ScrollView>

            {/* Animated picker — slides up from behind bottom bar */}
            {pickerVisible && (
              <Animated.View style={[styles.pickerContainer, { height: PICKER_HEIGHT, borderTopColor: t.border, backgroundColor: t.background }, pickerAnimStyle]}>
                <View style={styles.pickerHeader}>
                  <View>
                    <Text style={[styles.pickerTitle, { color: t.foreground }]}>Select Directory</Text>
                    <Text style={[styles.pickerPath, { color: t.mutedForeground }]} numberOfLines={1}>{pickerPath}</Text>
                  </View>
                  <Pressable onPress={hidePicker} style={styles.iconBtn}>
                    <ChevronDown size={20} color={t.mutedForeground} strokeWidth={1.8} />
                  </Pressable>
                </View>

                <ScrollView style={styles.pickerList} showsVerticalScrollIndicator={false}>
                  {pickerParent && (
                    <Pressable onPress={() => navigatePicker(pickerParent)} style={styles.pickerItem}>
                      <ChevronLeft size={16} color={t.mutedForeground} />
                      <Text style={[styles.pickerItemText, { color: t.mutedForeground }]}>Up</Text>
                    </Pressable>
                  )}
                  {pickerDirs.map(d => (
                    <Pressable key={d.path} onPress={() => navigatePicker(d.path)} style={styles.pickerItem}>
                      <Folder size={16} color={t.mutedForeground} strokeWidth={1.6} />
                      <Text style={[styles.pickerItemText, { color: t.foreground }]} numberOfLines={1}>{d.name}</Text>
                    </Pressable>
                  ))}
                </ScrollView>

                <Pressable
                  onPress={() => {
                    Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
                    hidePicker();
                    onNewSessionWithDir(pickerPath);
                  }}
                  style={[styles.pickerSelectBtn, { borderColor: t.mutedForeground }]}
                >
                  <Check size={16} color={t.mutedForeground} strokeWidth={2.5} />
                  <Text style={[styles.bottomBtnText, { color: t.mutedForeground }]}>Select This Directory</Text>
                </Pressable>
              </Animated.View>
            )}

            {/* Bottom bar: settings (left) ... arrow + new (right) */}
            <View style={[styles.bottomBar, { borderTopColor: t.border }]}>
              {onOpenSettings && (
                <Pressable
                  onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); onOpenSettings(); }}
                  style={styles.iconBtn}
                >
                  <Settings size={22} color={t.mutedForeground} strokeWidth={1.8} />
                </Pressable>
              )}

              <View style={styles.statusIcon}>
                {status === 'connected'
                  ? <Wifi size={16} color="#22c55e" strokeWidth={2} />
                  : <WifiOff size={16} color={status === 'connecting' ? '#f59e0b' : '#ef4444'} strokeWidth={2} />
                }
              </View>

              <View style={{ flex: 1 }} />

              <View style={styles.bottomRight}>
                <Pressable onPress={onClose} style={styles.iconBtn}>
                  <ArrowLeft size={22} color={t.mutedForeground} strokeWidth={1.8} />
                </Pressable>

                <Pressable
                  onPress={() => {
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
                    if (activeTab === 0) {
                      onNewSession();
                    } else {
                      showPicker();
                    }
                  }}
                  style={styles.iconBtn}
                >
                  {activeTab === 0
                    ? <SquarePlus size={22} color={t.mutedForeground} strokeWidth={1.8} />
                    : <FolderPlus size={22} color={t.mutedForeground} strokeWidth={1.8} />}
              </Pressable>
              </View>
            </View>
          </View>
        </Animated.View>
      </GestureDetector>
    </>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.4)',
    zIndex: 100,
  },
  drawer: {
    position: 'absolute',
    top: 0, left: 0, bottom: 0,
    zIndex: 101,
    overflow: 'hidden',
    borderRightWidth: StyleSheet.hairlineWidth,
    borderRightColor: 'rgba(255,255,255,0.1)',
  },
  content: {
    flex: 1,
    paddingTop: 54,
  },
  segmentWrap: {
    paddingHorizontal: 16,
    marginBottom: 12,
  },
  listArea: {
    flex: 1,
    paddingHorizontal: 12,
  },
  emptyText: {
    fontSize: 15,
    textAlign: 'center',
    marginTop: 40,
  },
  dirHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    paddingVertical: 10,
    paddingHorizontal: 8,
  },
  dirName: {
    flex: 1,
    fontSize: 14,
    fontWeight: '600',
  },
  dirCount: { fontSize: 12 },
  dirSessions: {
    paddingLeft: 12,
    marginBottom: 4,
  },
  sessionItem: {
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 10,
    marginBottom: 2,
  },
  sessionHeader: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    gap: 8,
  },
  sessionTitle: {
    flex: 1,
    fontSize: 14,
    fontWeight: '500',
    lineHeight: 19,
  },
  typePill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
  },
  typePillText: {
    fontSize: 10,
    fontWeight: '700',
    letterSpacing: 0.2,
    textTransform: 'uppercase',
  },
  sessionMeta: {
    flexDirection: 'row',
    gap: 8,
    marginTop: 3,
  },
  sessionTime: { fontSize: 12 },
  sessionModel: { fontSize: 12 },

  // Animated picker overlay — slides up from bottom
  pickerContainer: {
    position: 'absolute',
    left: 0, right: 0, bottom: 60, // sits above the bottom bar
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    paddingTop: 14,
    backgroundColor: 'rgba(11,17,25,0.96)',
  },
  pickerHeader: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    marginBottom: 8,
  },
  pickerTitle: {
    fontSize: 16,
    fontWeight: '700',
  },
  pickerPath: {
    fontSize: 11,
    marginTop: 2,
  },
  pickerList: {
    flex: 1,
  },
  pickerItem: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    paddingVertical: 9,
    paddingHorizontal: 4,
  },
  pickerItemText: {
    fontSize: 15,
    flex: 1,
  },
  pickerSelectBtn: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 8,
    paddingVertical: 11,
    borderRadius: 14,
    borderWidth: 1,
    marginTop: 6,
    marginBottom: 10,
  },

  // Bottom bar
  bottomBar: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 20,
    paddingVertical: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  bottomRight: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 16,
  },
  iconBtn: {
    padding: 6,
  },
  bottomBtnText: {
    color: '#fff',
    fontSize: 15,
    fontWeight: '600',
  },
  statusIcon: {
    marginLeft: 6,
  },
});
