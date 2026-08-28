import { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  FlatList,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";
import { Plus, Trash2 } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useStores } from "../../hooks/useStores";
import { SessionListSkeleton } from "../../components/ui/Skeleton";
import { GlassCard } from "../../components/ui/GlassCard";
import {
  beginAllModeSessionDeletionAdmission,
  clearDeletedSessionFromModeStoreGraphs,
  type SessionDeletionAdmission,
} from "../../components/chat-screen/sessionDeletionAdmission";
import type { SessionResponse } from "@mitsuro/api";

export default function SessionsScreen() {
  const { theme } = useThemeContext();
  const { client, isConnected, recoveryConnectionScope } = useConnection();
  const stores = useStores();
  const activeStoresRef = useRef(stores);
  activeStoresRef.current = stores;
  const activeRecoveryScopeRef = useRef<string | null>(
    recoveryConnectionScope,
  );
  activeRecoveryScopeRef.current = recoveryConnectionScope;
  useEffect(() => {
    activeRecoveryScopeRef.current = recoveryConnectionScope;
    return () => {
      if (activeRecoveryScopeRef.current === recoveryConnectionScope) {
        activeRecoveryScopeRef.current = null;
      }
    };
  }, [recoveryConnectionScope]);
  useEffect(() => {
    activeStoresRef.current = stores;
    return () => {
      if (activeStoresRef.current === stores) {
        activeStoresRef.current = null;
      }
    };
  }, [stores]);
  const [sessionSnapshot, setSessionSnapshot] = useState<
    {
      recoveryConnectionScope: string;
      sessions: SessionResponse[];
    } | null
  >(null);
  const sessions = recoveryConnectionScope &&
      sessionSnapshot?.recoveryConnectionScope === recoveryConnectionScope
    ? sessionSnapshot.sessions
    : [];
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);

  const loadSessions = useCallback(async () => {
    if (!client || !recoveryConnectionScope) return;
    const expectedScope = recoveryConnectionScope;
    try {
      const data = await client.getSessions();
      if (activeRecoveryScopeRef.current !== expectedScope) return;
      setSessionSnapshot({
        recoveryConnectionScope: expectedScope,
        sessions: data,
      });
    } catch {
      // silent
    } finally {
      if (activeRecoveryScopeRef.current === expectedScope) {
        setIsLoading(false);
        setIsRefreshing(false);
      }
    }
  }, [client, recoveryConnectionScope]);

  useEffect(() => {
    if (isConnected) loadSessions();
  }, [isConnected, loadSessions]);

  const handleRefresh = () => {
    setIsRefreshing(true);
    loadSessions();
  };

  const handleCreate = async () => {
    if (!client || !recoveryConnectionScope) return;
    const expectedScope = recoveryConnectionScope;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    try {
      const session = await client.createSession();
      if (activeRecoveryScopeRef.current !== expectedScope) return;
      setSessionSnapshot((previous) => ({
        recoveryConnectionScope: expectedScope,
        sessions: [
          session,
          ...(previous?.recoveryConnectionScope === expectedScope
            ? previous.sessions
            : []),
        ],
      }));
      router.navigate("/(tabs)");
    } catch {
      // silent
    }
  };

  const handleDelete = (session: SessionResponse) => {
    const expectedScope = recoveryConnectionScope;
    const expectedStores = stores;
    Alert.alert(
      "Delete Session",
      `Delete "${session.title || "Untitled"}"?`,
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: async () => {
            const isCurrentDeletionBoundary = () =>
              activeRecoveryScopeRef.current === expectedScope &&
              activeStoresRef.current === expectedStores;
            if (
              !expectedStores || !expectedScope ||
              !isCurrentDeletionBoundary()
            ) return;
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Heavy);
            let admission: SessionDeletionAdmission | null = null;
            try {
              admission = await beginAllModeSessionDeletionAdmission(
                expectedStores.modes,
                session.id,
              );
              if (!isCurrentDeletionBoundary()) {
                await admission.rollback();
                return;
              }
              const deleted = await expectedStores.sessions.getState()
                .deleteSession(session.id);
              if (!deleted) {
                await admission.rollback();
                if (!isCurrentDeletionBoundary()) return;
                Alert.alert(
                  "Delete unavailable",
                  expectedStores.sessions.getState().error ??
                    "The session could not be deleted safely.",
                );
                return;
              }
              clearDeletedSessionFromModeStoreGraphs(
                expectedStores.modes,
                activeRecoveryScopeRef.current === expectedScope
                  ? activeStoresRef.current?.modes ?? null
                  : null,
                session.id,
              );
              admission.commit();
              if (isCurrentDeletionBoundary()) {
                setSessionSnapshot((previous) => ({
                  recoveryConnectionScope: expectedScope,
                  sessions: previous?.recoveryConnectionScope === expectedScope
                    ? previous.sessions.filter((entry) =>
                      entry.id !== session.id
                    )
                    : [],
                }));
              }
            } catch (deleteError) {
              try {
                await admission?.rollback();
              } catch (rollbackError) {
                deleteError = rollbackError;
              }
              if (!isCurrentDeletionBoundary()) return;
              Alert.alert(
                "Delete incomplete",
                deleteError instanceof Error
                  ? deleteError.message
                  : "The session recovery record could not be cleared.",
              );
            }
          },
        },
      ],
    );
  };

  const t = theme.colors;

  const formatDate = (dateStr: string) => {
    const date = new Date(dateStr);
    const now = new Date();
    const diff = now.getTime() - date.getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return "Just now";
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  };

  return (
    <SafeAreaView style={[styles.container, { backgroundColor: t.background }]}>
      <View style={styles.header}>
        <Text style={[styles.title, { color: t.foreground }]}>Sessions</Text>
        <Pressable
          onPress={handleCreate}
          style={({ pressed }) => [
            styles.addButton,
            {
              backgroundColor: pressed ? t.userMessage + "cc" : t.userMessage,
            },
          ]}
        >
          <Plus size={20} color={t.onAccent} strokeWidth={2.5} />
        </Pressable>
      </View>

      {isLoading ? <SessionListSkeleton /> : (
        <FlatList
          data={sessions}
          keyExtractor={(s) => s.id}
          renderItem={({ item }) => (
            <Pressable
              onPress={() => {
                Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                router.navigate("/(tabs)");
              }}
            >
              <GlassCard style={styles.card}>
                <View style={styles.cardRow}>
                  <View style={styles.cardContent}>
                    <Text
                      style={[styles.cardTitle, { color: t.foreground }]}
                      numberOfLines={1}
                    >
                      {item.title || "Untitled"}
                    </Text>
                    <View style={styles.cardMeta}>
                      <Text
                        style={[styles.cardDate, { color: t.mutedForeground }]}
                      >
                        {formatDate(item.updated_at)}
                      </Text>
                      {item.token_count != null && (
                        <Text
                          style={[styles.cardTokens, {
                            color: t.mutedForeground,
                          }]}
                        >
                          {(item.token_count / 1000).toFixed(0)}k tokens
                        </Text>
                      )}
                    </View>
                  </View>
                  <Pressable
                    onPress={() => handleDelete(item)}
                    hitSlop={12}
                    style={styles.deleteBtn}
                  >
                    <Trash2
                      size={18}
                      color={t.mutedForeground}
                      strokeWidth={1.5}
                    />
                  </Pressable>
                </View>
              </GlassCard>
            </Pressable>
          )}
          contentContainerStyle={styles.list}
          refreshControl={
            <RefreshControl
              refreshing={isRefreshing}
              onRefresh={handleRefresh}
              tintColor={t.userMessage}
            />
          }
          ListEmptyComponent={
            <View style={styles.center}>
              <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
                No sessions yet
              </Text>
            </View>
          }
        />
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  header: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingHorizontal: 20,
    paddingVertical: 16,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  addButton: {
    width: 36,
    height: 36,
    borderRadius: 18,
    justifyContent: "center",
    alignItems: "center",
  },
  center: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  list: {
    paddingHorizontal: 16,
    gap: 10,
    paddingBottom: 100,
  },
  card: {
    marginBottom: 0,
  },
  cardRow: {
    flexDirection: "row",
    alignItems: "center",
  },
  cardContent: {
    flex: 1,
    gap: 4,
  },
  cardTitle: {
    fontSize: 17,
    fontWeight: "600",
  },
  cardMeta: {
    flexDirection: "row",
    gap: 12,
  },
  cardDate: {
    fontSize: 13,
  },
  cardTokens: {
    fontSize: 13,
  },
  deleteBtn: {
    padding: 8,
  },
  emptyText: {
    fontSize: 17,
  },
});
