import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import type { AgentMemory, MemoryType } from "@krusty/api";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { GlassCard } from "../ui/GlassCard";
import { MakoInsightCard } from "./MakoInsightCard";
import { MakoKnowledgeScopeToggle } from "./MakoKnowledgeScopeToggle";
import { MakoTopNav } from "./MakoTopNav";
import { formatProjectLabel, formatRelativeTime, formatTimestamp } from "./utils";
import { useMakoMemories } from "./hooks/useMakoMemories";

interface MakoMemoryViewProps {
  workspaceDirectory?: string | null;
  state: ReturnType<typeof useMakoMemories>;
}

const MEMORY_FILTERS: Array<{ id: MemoryType | "all"; label: string }> = [
  { id: "all", label: "All" },
  { id: "project", label: "Project" },
  { id: "user", label: "User" },
  { id: "feedback", label: "Feedback" },
  { id: "reference", label: "Reference" },
];

function formatMemoryTypeLabel(memoryType: MemoryType): string {
  switch (memoryType) {
    case "project":
      return "Project";
    case "user":
      return "User";
    case "feedback":
      return "Feedback";
    case "reference":
      return "Reference";
  }
}

function matchesMemoryQuery(memory: AgentMemory, query: string): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) {
    return true;
  }

  return [
    memory.title,
    memory.content,
    memory.project_dir ?? "",
    memory.memory_type,
  ]
    .join(" ")
    .toLowerCase()
    .includes(normalizedQuery);
}

function topMemoryTypeLabel(memories: AgentMemory[]): { value: string; detail: string } {
  const counts = new Map<MemoryType, number>();
  for (const memory of memories) {
    counts.set(memory.memory_type, (counts.get(memory.memory_type) ?? 0) + 1);
  }

  const topType = [...counts.entries()].sort((left, right) => {
    if (right[1] !== left[1]) {
      return right[1] - left[1];
    }
    return left[0].localeCompare(right[0]);
  })[0];

  if (!topType) {
    return { value: "None", detail: "No memories yet" };
  }

  return {
    value: formatMemoryTypeLabel(topType[0]),
    detail: `${topType[1]} entr${topType[1] === 1 ? "y" : "ies"}`,
  };
}

function MemoryCard({
  memory,
  selected,
  onPress,
}: {
  memory: AgentMemory;
  selected: boolean;
  onPress: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <Pressable onPress={onPress}>
      <GlassCard style={styles.card} elevated={selected}>
        <View style={styles.cardHeader}>
          <Text style={[styles.cardTitle, { color: t.foreground }]} numberOfLines={2}>
            {memory.title}
          </Text>
          <Text style={[styles.cardType, { color: t.userMessage }]}>
            {formatMemoryTypeLabel(memory.memory_type)}
          </Text>
        </View>

        <Text
          style={[styles.cardSummary, { color: t.mutedForeground }]}
          numberOfLines={4}
        >
          {memory.content}
        </Text>

        <View style={styles.cardFooter}>
          <Text style={[styles.cardMeta, { color: t.mutedForeground }]}>
            {formatProjectLabel(memory.project_dir)}
          </Text>
          <Text style={[styles.cardMeta, { color: t.mutedForeground }]}>
            {formatRelativeTime(memory.updated_at)}
          </Text>
        </View>
      </GlassCard>
    </Pressable>
  );
}

function MemoryDetailPane({
  memory,
}: {
  memory: AgentMemory | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (!memory) {
    return (
      <GlassCard style={styles.detailCard} elevated>
        <Text style={[styles.detailTitle, { color: t.foreground }]}>
          No memory selected
        </Text>
        <Text style={[styles.detailBody, { color: t.mutedForeground }]}>
          Memories are the durable facts, decisions, and references Mako should keep carrying across runs.
        </Text>
      </GlassCard>
    );
  }

  return (
    <GlassCard style={styles.detailCard} elevated>
      <Text style={[styles.detailTitle, { color: t.foreground }]}>
        {memory.title}
      </Text>

      <View style={styles.detailMetaRow}>
        <Text style={[styles.detailMeta, { color: t.userMessage }]}>
          {formatMemoryTypeLabel(memory.memory_type)}
        </Text>
        <Text style={[styles.detailMeta, { color: t.mutedForeground }]}>
          {formatProjectLabel(memory.project_dir)}
        </Text>
      </View>

      <Text style={[styles.detailBody, { color: t.foreground }]}>
        {memory.content}
      </Text>

      <View style={styles.detailTimeRow}>
        <Text style={[styles.detailMeta, { color: t.mutedForeground }]}>
          Created {formatTimestamp(memory.created_at)}
        </Text>
        <Text style={[styles.detailMeta, { color: t.mutedForeground }]}>
          Updated {formatTimestamp(memory.updated_at)}
        </Text>
      </View>
    </GlassCard>
  );
}

export function MakoMemoryView({
  workspaceDirectory,
  state,
}: MakoMemoryViewProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const [query, setQuery] = useState("");

  const visibleMemories = state.memories.filter((memory) =>
    matchesMemoryQuery(memory, query),
  );
  const visibleMemoryIds = visibleMemories.map((memory) => memory.id).join("|");
  const uniqueProjects = new Set(
    visibleMemories
      .map((memory) => memory.project_dir)
      .filter((projectDir): projectDir is string => Boolean(projectDir)),
  ).size;
  const topType = topMemoryTypeLabel(visibleMemories);

  useEffect(() => {
    if (!state.selectedMemoryId && visibleMemories.length === 0) {
      state.clearSelection();
      return;
    }

    if (
      state.selectedMemoryId &&
      !visibleMemories.some((memory) => memory.id === state.selectedMemoryId)
    ) {
      state.clearSelection();
      return;
    }

    if (isDesktop && !state.selectedMemoryId && visibleMemories.length > 0) {
      state.setSelectedMemoryId(visibleMemories[0].id);
    }
  }, [
    state.clearSelection,
    isDesktop,
    state.selectedMemoryId,
    state.setSelectedMemoryId,
    visibleMemoryIds,
  ]);

  if (state.isLoading && state.memories.length === 0) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={t.userMessage} />
      </View>
    );
  }

  const listContent = (
    <>
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Memory is where reports turn into durable working knowledge. Keep it concise enough to carry forward, not verbose enough to become another report archive.
      </Text>

      <View style={styles.metricsRow}>
        <MakoInsightCard
          label="Memories"
          value={String(visibleMemories.length)}
          detail={query.trim() ? "matching this filter" : "visible in this scope"}
          style={styles.metricCard}
        />
        <MakoInsightCard
          label="Projects"
          value={String(uniqueProjects)}
          detail={state.scope === "workspace" ? "in this workspace" : "represented here"}
          style={styles.metricCard}
        />
      </View>

      <View style={styles.metricsRow}>
        <MakoInsightCard
          label="Latest"
          value={formatRelativeTime(visibleMemories[0]?.updated_at)}
          detail="most recently updated"
          style={styles.metricCard}
        />
        <MakoInsightCard
          label="Top type"
          value={topType.value}
          detail={topType.detail}
          tone={topType.value === "None" ? "default" : "accent"}
          style={styles.metricCard}
        />
      </View>

      <MakoKnowledgeScopeToggle
        activeScope={state.scope}
        workspaceDirectory={workspaceDirectory}
        allLabel="All memories"
        allHint="across every workspace"
        onSelect={state.setScope}
      />

      <MakoTopNav
        items={MEMORY_FILTERS}
        active={state.typeFilter}
        onSelect={state.setTypeFilter}
      />

      <TextInput
        value={query}
        onChangeText={setQuery}
        placeholder="Search memories"
        placeholderTextColor={t.mutedForeground}
        style={[
          styles.searchInput,
          {
            color: t.foreground,
            borderColor: t.border,
            backgroundColor: t.card,
          },
        ]}
      />

      {state.error ? (
        <Text style={[styles.error, { color: t.error }]}>{state.error}</Text>
      ) : null}

      {visibleMemories.length === 0 ? (
        <GlassCard style={styles.emptyCard}>
          <Text style={[styles.emptyTitle, { color: t.foreground }]}>
            {state.memories.length === 0 ? "No memories yet" : "No matching memories"}
          </Text>
          <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
            {state.memories.length === 0
              ? "Promote an important report or let future automation accumulate durable knowledge here."
              : "Try a different search term or memory type."}
          </Text>
        </GlassCard>
      ) : (
        visibleMemories.map((memory) => (
          <MemoryCard
            key={memory.id}
            memory={memory}
            selected={memory.id === state.selectedMemoryId}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              state.setSelectedMemoryId(memory.id);
            }}
          />
        ))
      )}
    </>
  );

  if (isDesktop) {
    return (
      <View style={styles.desktopLayout}>
        <ScrollView
          style={styles.desktopColumn}
          contentContainerStyle={styles.content}
          refreshControl={
            <RefreshControl
              refreshing={state.isRefreshing}
              onRefresh={() => {
                void state.refresh();
              }}
              tintColor={t.userMessage}
            />
          }
          showsVerticalScrollIndicator={false}
        >
          {listContent}
        </ScrollView>

        <ScrollView
          style={styles.desktopDetailColumn}
          contentContainerStyle={styles.desktopDetailContent}
          showsVerticalScrollIndicator={false}
        >
          <MemoryDetailPane memory={state.selectedMemory} />
        </ScrollView>
      </View>
    );
  }

  if (state.selectedMemoryId) {
    return (
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        refreshControl={
          <RefreshControl
            refreshing={state.isRefreshing}
            onRefresh={() => {
              void state.refresh();
            }}
            tintColor={t.userMessage}
          />
        }
        showsVerticalScrollIndicator={false}
      >
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            state.clearSelection();
          }}
          style={styles.backButton}
        >
          <Text style={[styles.backLabel, { color: t.userMessage }]}>
            Back to memory
          </Text>
        </Pressable>

        <MemoryDetailPane memory={state.selectedMemory} />
      </ScrollView>
    );
  }

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={state.isRefreshing}
          onRefresh={() => {
            void state.refresh();
          }}
          tintColor={t.userMessage}
        />
      }
      showsVerticalScrollIndicator={false}
    >
      {listContent}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  desktopLayout: {
    flex: 1,
    flexDirection: "row",
    gap: 16,
    paddingHorizontal: 16,
    paddingBottom: 24,
  },
  desktopColumn: {
    flex: 1,
  },
  desktopDetailColumn: {
    flex: 1.1,
  },
  desktopDetailContent: {
    paddingBottom: 4,
  },
  scroll: {
    flex: 1,
  },
  content: {
    paddingBottom: 28,
    paddingHorizontal: 16,
    gap: 12,
  },
  loading: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  metricsRow: {
    flexDirection: "row",
    gap: 12,
  },
  metricCard: {
    flex: 1,
  },
  searchInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 16,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 14,
  },
  error: {
    fontSize: 13,
    lineHeight: 18,
  },
  card: {
    marginBottom: 0,
  },
  cardHeader: {
    flexDirection: "row",
    gap: 12,
    alignItems: "flex-start",
  },
  cardTitle: {
    flex: 1,
    fontSize: 16,
    fontWeight: "700",
    lineHeight: 22,
  },
  cardType: {
    fontSize: 12,
    fontWeight: "700",
  },
  cardSummary: {
    marginTop: 10,
    fontSize: 13,
    lineHeight: 18,
  },
  cardFooter: {
    marginTop: 14,
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
  },
  cardMeta: {
    fontSize: 12,
    fontWeight: "500",
  },
  emptyCard: {
    marginBottom: 0,
  },
  emptyTitle: {
    fontSize: 16,
    fontWeight: "700",
  },
  emptyBody: {
    marginTop: 8,
    fontSize: 13,
    lineHeight: 18,
  },
  backButton: {
    alignSelf: "flex-start",
    paddingVertical: 6,
  },
  backLabel: {
    fontSize: 13,
    fontWeight: "700",
  },
  detailCard: {
    marginBottom: 0,
  },
  detailTitle: {
    fontSize: 22,
    fontWeight: "700",
    lineHeight: 28,
  },
  detailMetaRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
    marginTop: 10,
  },
  detailMeta: {
    fontSize: 12,
    lineHeight: 16,
  },
  detailBody: {
    marginTop: 18,
    fontSize: 14,
    lineHeight: 21,
  },
  detailTimeRow: {
    gap: 8,
    marginTop: 18,
  },
});
