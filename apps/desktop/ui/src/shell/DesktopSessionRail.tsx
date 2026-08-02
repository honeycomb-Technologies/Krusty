import { useMemo, useState } from 'react';
import { Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { ChevronDown, ChevronRight, Folder, FolderOpen, Plus } from 'lucide-react-native';
import type { SessionResponse } from '@mitsuro/api';
import { useThemeContext } from '@mobile/hooks/useTheme';
import type { DesktopPlane } from './types';

function formatTime(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function dirDisplayName(path: string): string {
  if (path === 'Neutral') return 'General';
  const parts = path.split('/').filter(Boolean);
  return parts.slice(-2).join('/') || path;
}

export function DesktopSessionRail({
  plane,
  sessions,
  activeSessionId,
  onSelectSession,
  onDeleteSession,
  onNewSession,
  onOpenProject,
}: {
  plane: DesktopPlane;
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onDeleteSession: (id: string) => void;
  onNewSession: () => void;
  onOpenProject: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());

  const chatSessions = useMemo(
    () =>
      sessions
        .filter((session) => session.session_type === 'chat')
        .sort((a, b) => +new Date(b.updated_at) - +new Date(a.updated_at)),
    [sessions],
  );

  const codeGroups = useMemo(() => {
    const groups = new Map<string, SessionResponse[]>();
    for (const session of sessions) {
      if (session.session_type !== 'code') continue;
      const dir = session.project_dir ?? session.working_dir ?? 'Neutral';
      const list = groups.get(dir) ?? [];
      list.push(session);
      groups.set(dir, list);
    }
    for (const list of groups.values()) {
      list.sort((a, b) => +new Date(b.updated_at) - +new Date(a.updated_at));
    }
    return Array.from(groups.entries()).sort(([a], [b]) => {
      if (a === 'Neutral') return -1;
      if (b === 'Neutral') return 1;
      return a.localeCompare(b);
    });
  }, [sessions]);

  const hiveSessions = useMemo(
    () =>
      sessions
        .filter((session) => session.session_type === 'hive')
        .sort((a, b) => +new Date(b.updated_at) - +new Date(a.updated_at)),
    [sessions],
  );

  const renderSession = (session: SessionResponse, accent = t.userMessage) => {
    const active = session.id === activeSessionId;
    return (
      <Pressable
        key={session.id}
        onPress={() => onSelectSession(session)}
        onLongPress={() => onDeleteSession(session.id)}
        style={[
          styles.sessionRow,
          active && {
            backgroundColor: `${accent}14`,
            borderColor: `${accent}33`,
          },
        ]}
      >
        <Text
          style={[styles.sessionTitle, { color: active ? accent : t.foreground }]}
          numberOfLines={2}
        >
          {session.title || 'Untitled'}
        </Text>
        <View style={styles.sessionMeta}>
          <Text style={[styles.metaText, { color: t.mutedForeground }]}>
            {formatTime(session.updated_at)}
          </Text>
          {session.model ? (
            <Text style={[styles.metaText, { color: t.mutedForeground }]} numberOfLines={1}>
              {session.model.length > 18 ? `${session.model.slice(0, 18)}…` : session.model}
            </Text>
          ) : null}
        </View>
      </Pressable>
    );
  };

  if (plane === 'chat') {
    return (
      <View style={styles.root}>
        <View style={styles.actions}>
          <Pressable
            onPress={onNewSession}
            style={[styles.primaryBtn, { borderColor: t.border, backgroundColor: t.glass.background }]}
          >
            <Plus size={14} color={t.foreground} />
            <Text style={[styles.primaryBtnText, { color: t.foreground }]}>New chat</Text>
          </Pressable>
        </View>
        <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
          {chatSessions.length === 0 ? (
            <Text style={[styles.empty, { color: t.mutedForeground }]}>No chats</Text>
          ) : (
            chatSessions.map((session) => renderSession(session))
          )}
        </ScrollView>
      </View>
    );
  }

  if (plane === 'code') {
    return (
      <View style={styles.root}>
        <View style={styles.actions}>
          <Pressable
            onPress={onOpenProject}
            style={[styles.primaryBtn, { borderColor: t.border, backgroundColor: t.glass.background }]}
          >
            <FolderOpen size={14} color={t.foreground} />
            <Text style={[styles.primaryBtnText, { color: t.foreground }]}>Open project</Text>
          </Pressable>
          <Pressable
            onPress={onNewSession}
            style={[styles.secondaryBtn, { borderColor: t.border }]}
          >
            <Plus size={14} color={t.mutedForeground} />
          </Pressable>
        </View>
        <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
          {codeGroups.length === 0 ? (
            <Text style={[styles.empty, { color: t.mutedForeground }]}>
              No projects
            </Text>
          ) : (
            codeGroups.map(([dir, dirSessions]) => {
              const isOpen = expanded.has(dir) || dirSessions.some((s) => s.id === activeSessionId);
              return (
                <View key={dir} style={styles.group}>
                  <Pressable
                    onPress={() =>
                      setExpanded((current) => {
                        const next = new Set(current);
                        if (next.has(dir)) next.delete(dir);
                        else next.add(dir);
                        return next;
                      })
                    }
                    style={styles.groupHeader}
                  >
                    {isOpen ? (
                      <FolderOpen size={15} color={t.thinking} />
                    ) : (
                      <Folder size={15} color={t.mutedForeground} />
                    )}
                    <Text
                      style={[styles.groupTitle, { color: isOpen ? t.foreground : t.mutedForeground }]}
                      numberOfLines={1}
                    >
                      {dirDisplayName(dir)}
                    </Text>
                    <Text style={[styles.metaText, { color: t.mutedForeground }]}>
                      {dirSessions.length}
                    </Text>
                    {isOpen ? (
                      <ChevronDown size={14} color={t.mutedForeground} />
                    ) : (
                      <ChevronRight size={14} color={t.mutedForeground} />
                    )}
                  </Pressable>
                  {isOpen ? (
                    <View style={styles.groupBody}>
                      {dirSessions.map((session) => renderSession(session, t.thinking))}
                    </View>
                  ) : null}
                </View>
              );
            })
          )}
        </ScrollView>
      </View>
    );
  }

  return (
    <View style={styles.root}>
      <View style={styles.actions}>
        <Pressable
          onPress={onNewSession}
          style={[styles.primaryBtn, { borderColor: t.border, backgroundColor: t.glass.background }]}
        >
          <Plus size={14} color={t.foreground} />
          <Text style={[styles.primaryBtnText, { color: t.foreground }]}>New Hive thread</Text>
        </Pressable>
      </View>
      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {hiveSessions.length === 0 ? (
          <Text style={[styles.empty, { color: t.mutedForeground }]}>
            No Hive threads
          </Text>
        ) : (
          hiveSessions.map((session) => renderSession(session, t.success))
        )}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  actions: {
    paddingHorizontal: 12,
    paddingTop: 10,
    paddingBottom: 8,
    flexDirection: 'row',
    gap: 8,
  },
  primaryBtn: {
    flex: 1,
    minHeight: 34,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 10,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
  },
  primaryBtnText: {
    fontSize: 12,
    fontWeight: '700',
  },
  secondaryBtn: {
    width: 34,
    height: 34,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  list: { flex: 1 },
  listContent: {
    paddingHorizontal: 10,
    paddingBottom: 18,
    gap: 4,
  },
  empty: {
    fontSize: 12,
    lineHeight: 17,
    paddingHorizontal: 6,
    paddingTop: 10,
  },
  sessionRow: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'transparent',
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 9,
    gap: 4,
  },
  sessionTitle: {
    fontSize: 13,
    fontWeight: '600',
    lineHeight: 17,
  },
  sessionMeta: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    gap: 8,
  },
  metaText: {
    fontSize: 11,
  },
  group: {
    marginBottom: 4,
  },
  groupHeader: {
    minHeight: 34,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 6,
  },
  groupTitle: {
    flex: 1,
    fontSize: 12,
    fontWeight: '700',
  },
  groupBody: {
    paddingLeft: 8,
    gap: 2,
  },
});
