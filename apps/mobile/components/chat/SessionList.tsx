import { useState, useMemo, useCallback, useEffect, useRef } from 'react';
import {
  View,
  Text,
  Pressable,
  ScrollView,
  StyleSheet,
} from 'react-native';
import { Folder, FolderOpen, ChevronRight, ChevronDown, ChevronLeft, Check } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import type { SessionResponse } from '@krusty/api';
import type { MakoTopLevelView } from '../mako/types';

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

export interface SessionListProps {
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onDeleteSession: (id: string) => void;
  onNewSession: () => void;
  onNewSessionWithDir: (path: string) => void;
  activeTab: number;
  onTabChange: (index: number) => void;
  activeMakoView?: MakoTopLevelView;
  onSelectMakoView?: (view: MakoTopLevelView) => void;
  showPicker?: boolean;
  onPickerDone?: () => void;
}

export function SessionList({
  sessions,
  activeSessionId,
  onSelectSession,
  onDeleteSession,
  onNewSession,
  onNewSessionWithDir,
  activeTab,
  onTabChange,
  activeMakoView,
  onSelectMakoView,
  showPicker,
  onPickerDone,
}: SessionListProps) {
  const { theme } = useThemeContext();
  const { client } = useConnection();
  const t = theme.colors;
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

  // Directory picker state
  const [pickerPath, setPickerPath] = useState('');
  const [pickerParent, setPickerParent] = useState<string | null>(null);
  const [pickerDirs, setPickerDirs] = useState<DirEntry[]>([]);
  const [pickerReady, setPickerReady] = useState(false);
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

  useEffect(() => {
    if (client && showPicker && !pickerReady) {
      client.browseDirectories().then(result => {
        const entry: DirCache = { current: result.current, parent: result.parent, directories: result.directories };
        setDirCache('', entry);
        setDirCache(result.current, entry);
        setPickerPath(result.current);
        setPickerParent(result.parent);
        setPickerDirs(result.directories);
        setPickerReady(true);
      }).catch(() => {});
    }
  }, [client, pickerReady, showPicker]);

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
      setDirCache(path, entry);
      setDirCache(result.current, entry);
      setPickerPath(result.current);
      setPickerParent(result.parent);
      setPickerDirs(result.directories);
    } catch { /* keep current view */ }
  }, [client]);

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
    <View style={styles.container}>
      <ScrollView style={styles.listArea} showsVerticalScrollIndicator={false}>
        <Text style={[styles.sectionLabel, { color: t.mutedForeground }]}>Conversations</Text>
        {chatSessions.length === 0
          ? <Text style={[styles.sectionEmpty, { color: t.mutedForeground }]}>No conversations</Text>
          : chatSessions.map(renderSessionItem)}
        <Text style={[styles.sectionLabel, styles.codeSection, { color: t.mutedForeground }]}>Code</Text>
        {renderDirAccordion(codeDirGroups, t.thinking)}
      </ScrollView>

      {/* Inline directory picker */}
      {showPicker && pickerReady && (
        <View style={[styles.pickerContainer, { borderTopColor: t.border, backgroundColor: t.background }]}>
          <View style={styles.pickerHeader}>
            <View>
              <Text style={[styles.pickerTitle, { color: t.foreground }]}>Select Directory</Text>
              <Text style={[styles.pickerPath, { color: t.mutedForeground }]} numberOfLines={1}>{pickerPath}</Text>
            </View>
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
              onPickerDone?.();
              onNewSessionWithDir(pickerPath);
            }}
            style={[styles.pickerSelectBtn, { borderColor: t.mutedForeground }]}
          >
            <Check size={16} color={t.mutedForeground} strokeWidth={2.5} />
            <Text style={[styles.selectBtnText, { color: t.mutedForeground }]}>Select This Directory</Text>
          </Pressable>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  listArea: { flex: 1, paddingHorizontal: 12 },
  sectionLabel: { fontSize: 12, fontWeight: '700', letterSpacing: 0.7, textTransform: 'uppercase', paddingHorizontal: 8, marginBottom: 6 },
  codeSection: { marginTop: 18 },
  sectionEmpty: { fontSize: 13, paddingHorizontal: 8, paddingVertical: 8 },
  emptyText: { fontSize: 15, textAlign: 'center', marginTop: 40 },
  dirHeader: { flexDirection: 'row', alignItems: 'center', gap: 8, paddingVertical: 10, paddingHorizontal: 8 },
  dirName: { flex: 1, fontSize: 14, fontWeight: '600' },
  dirCount: { fontSize: 12 },
  dirSessions: { paddingLeft: 12, marginBottom: 4 },
  sessionItem: { paddingHorizontal: 12, paddingVertical: 10, borderRadius: 10, marginBottom: 2 },
  sessionTitle: { flex: 1, fontSize: 14, fontWeight: '500', lineHeight: 19 },
  sessionMeta: { flexDirection: 'row', gap: 8, marginTop: 3 },
  sessionTime: { fontSize: 12 },
  sessionModel: { fontSize: 12 },
  makoItem: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    paddingHorizontal: 12,
    paddingVertical: 12,
    borderRadius: 10,
    marginBottom: 2,
  },
  makoCopy: {
    flex: 1,
    minWidth: 0,
  },
  makoTitle: {
    fontSize: 14,
    fontWeight: '500',
  },
  makoDetail: {
    marginTop: 3,
    fontSize: 12,
  },
  makoBadge: {
    minWidth: 18,
    height: 18,
    borderRadius: 9,
    paddingHorizontal: 5,
    alignItems: 'center',
    justifyContent: 'center',
  },
  makoBadgeText: {
    color: '#081018',
    fontSize: 11,
    fontWeight: '700',
  },
  pickerContainer: { borderTopWidth: StyleSheet.hairlineWidth, paddingHorizontal: 16, paddingTop: 14 },
  pickerHeader: { flexDirection: 'row', alignItems: 'flex-start', justifyContent: 'space-between', marginBottom: 8 },
  pickerTitle: { fontSize: 16, fontWeight: '700' },
  pickerPath: { fontSize: 11, marginTop: 2 },
  pickerList: { maxHeight: 200 },
  pickerItem: { flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 9, paddingHorizontal: 4 },
  pickerItemText: { fontSize: 15, flex: 1 },
  pickerSelectBtn: { flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8, paddingVertical: 11, borderRadius: 14, borderWidth: 1, marginTop: 6, marginBottom: 10 },
  selectBtnText: { fontSize: 15, fontWeight: '600' },
});
