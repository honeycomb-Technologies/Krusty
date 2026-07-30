import { useEffect, useState } from "react";
import {
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import type { Report } from "@krusty/api";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { DetailPaneSkeleton, ListRowsSkeleton } from "../ui/Skeleton";
import { ReportDetailContent } from "../reports/ReportDetailContent";
import { MakoInsightCard } from "./MakoInsightCard";
import { MakoKnowledgeScopeToggle } from "./MakoKnowledgeScopeToggle";
import { MakoMemoryView } from "./MakoMemoryView";
import { MakoTopNav } from "./MakoTopNav";
import { useMakoMemories } from "./hooks/useMakoMemories";
import { useMakoReports } from "./hooks/useMakoReports";
import { formatProjectLabel, formatRelativeTime } from "./utils";
import type { MakoKnowledgeView } from "./types";

interface MakoReportsViewProps {
  workspaceDirectory?: string | null;
}

interface PromotionState {
  message: string;
  tone: "success" | "accent" | "danger";
}

function topTagLabel(
  reports: Array<{
    tags: string[];
  }>,
): { value: string; detail: string } {
  const counts = new Map<string, number>();

  for (const report of reports) {
    for (const tag of report.tags) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }

  const topTag = [...counts.entries()].sort((left, right) => {
    if (right[1] !== left[1]) {
      return right[1] - left[1];
    }
    return left[0].localeCompare(right[0]);
  })[0];

  if (!topTag) {
    return { value: "None", detail: "No tags yet" };
  }

  return {
    value: topTag[0],
    detail: `${topTag[1]} report${topTag[1] === 1 ? "" : "s"}`,
  };
}

function ReportCard({
  report,
  selected,
  onPress,
}: {
  report: {
    id: string;
    title: string;
    summary: string;
    tags: string[];
    created_at: string;
    project_dir?: string;
  };
  selected: boolean;
  onPress: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <Pressable onPress={onPress}>
      <View
        style={[
          styles.card,
          {
            borderColor: t.border,
            backgroundColor: selected ? t.glass.backgroundElevated : "transparent",
          },
        ]}
      >
        <Text style={[styles.title, { color: t.foreground }]} numberOfLines={2}>
          {report.title}
        </Text>
        <Text style={[styles.summary, { color: t.mutedForeground }]} numberOfLines={3}>
          {report.summary || "No summary available."}
        </Text>

        <View style={styles.metaRow}>
          <Text style={[styles.date, { color: t.mutedForeground }]}>
            {formatProjectLabel(report.project_dir)}
          </Text>
          <Text style={[styles.date, { color: t.mutedForeground }]}>
            {formatRelativeTime(report.created_at)}
          </Text>
        </View>

        <View style={styles.footer}>
          <Text style={[styles.tags, { color: t.userMessage }]}>
            {report.tags.length > 0
              ? report.tags.slice(0, 3).join(" • ")
              : "No tags"}
          </Text>
          {selected ? (
            <Text style={[styles.selectedLabel, { color: t.userMessage }]}>
              Open
            </Text>
          ) : null}
        </View>
      </View>
    </Pressable>
  );
}

function ReportDetailPane({
  isLoading,
  report,
  isPromoting,
  promotionState,
  onPromote,
}: {
  isLoading: boolean;
  report: Report | null;
  isPromoting: boolean;
  promotionState: PromotionState | null;
  onPromote: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (isLoading) {
    return (
      <View style={styles.loading}>
        <DetailPaneSkeleton />
      </View>
    );
  }

  if (!report) {
    return (
      <View style={[styles.detailCard, { borderColor: t.border }]}>
        <Text style={[styles.detailTitle, { color: t.foreground }]}>
          No report selected
        </Text>
        <Text style={[styles.detailCopy, { color: t.mutedForeground }]}>
          Open any report to inspect the written brief, sources, and project context.
        </Text>
      </View>
    );
  }

  const promotionColor =
    promotionState?.tone === "danger"
      ? t.error
      : promotionState?.tone === "success"
        ? t.success
        : t.userMessage;

  return (
    <View style={[styles.detailCard, { borderColor: t.border }]}>
      <Text style={[styles.detailTitle, { color: t.foreground }]}>{report.title}</Text>
      {report.summary ? (
        <Text style={[styles.detailCopy, { color: t.mutedForeground }]}>
          {report.summary}
        </Text>
      ) : null}

      <Pressable
        onPress={onPromote}
        disabled={isPromoting}
        style={[
          styles.promoteButton,
          {
            borderColor: t.border,
            backgroundColor: `${t.userMessage}14`,
            opacity: isPromoting ? 0.7 : 1,
          },
        ]}
      >
        <Text style={[styles.promoteLabel, { color: t.userMessage }]}>
          {isPromoting ? "Promoting..." : "Promote to project memory"}
        </Text>
      </Pressable>

      {promotionState ? (
        <Text style={[styles.promotionMessage, { color: promotionColor }]}>
          {promotionState.message}
        </Text>
      ) : null}

      <ReportDetailContent report={report} />
    </View>
  );
}

export function MakoReportsView({ workspaceDirectory }: MakoReportsViewProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const { client } = useConnection();
  const t = theme.colors;
  const [knowledgeView, setKnowledgeView] = useState<MakoKnowledgeView>("recent");
  const [query, setQuery] = useState("");
  const [isPromoting, setIsPromoting] = useState(false);
  const [promotionState, setPromotionState] = useState<PromotionState | null>(null);
  const reports = useMakoReports(knowledgeView === "recent", workspaceDirectory, query);
  const memories = useMakoMemories(knowledgeView === "memory", workspaceDirectory);
  const visibleReports = reports.reports;
  const visibleReportIds = visibleReports.map((report) => report.id).join("|");
  const uniqueProjects = new Set(
    visibleReports
      .map((report) => report.project_dir)
      .filter((projectDir): projectDir is string => Boolean(projectDir)),
  ).size;
  const topTag = topTagLabel(visibleReports);

  useEffect(() => {
    setPromotionState(null);
  }, [reports.selectedReportId]);

  useEffect(() => {
    if (knowledgeView !== "recent") {
      return;
    }

    if (!reports.selectedReportId && visibleReports.length === 0) {
      reports.clearSelection();
      return;
    }

    if (
      reports.selectedReportId &&
      !visibleReports.some((report) => report.id === reports.selectedReportId)
    ) {
      reports.clearSelection();
      return;
    }

    if (isDesktop && !reports.selectedReportId && visibleReports.length > 0) {
      void reports.selectReport(visibleReports[0].id);
    }
  }, [
    knowledgeView,
    isDesktop,
    reports.clearSelection,
    reports.selectReport,
    reports.selectedReportId,
    visibleReportIds,
  ]);

  const handlePromote = async () => {
    if (!client || !reports.selectedReport) {
      return;
    }

    setIsPromoting(true);
    setPromotionState(null);
    try {
      const response = await client.promoteReportToMemory(reports.selectedReport.id, {
        memoryType: "project",
      });
      await memories.refresh();
      setPromotionState({
        tone: response.created ? "success" : "accent",
        message: response.created
          ? "Saved to project memory."
          : "Updated existing project memory.",
      });
    } catch (error) {
      setPromotionState({
        tone: "danger",
        message:
          error instanceof Error
            ? error.message
            : "Failed to promote this report.",
      });
    } finally {
      setIsPromoting(false);
    }
  };

  if (knowledgeView === "memory") {
    return (
      <>
        <MakoTopNav
          items={[
            { id: "recent", label: "Recent" },
            { id: "memory", label: "Memory" },
          ]}
          active={knowledgeView}
          onSelect={setKnowledgeView}
        />
        <MakoMemoryView workspaceDirectory={workspaceDirectory} state={memories} />
      </>
    );
  }

  const listContent = (
    <>
      <MakoTopNav
        items={[
          { id: "recent", label: "Recent" },
          { id: "memory", label: "Memory" },
        ]}
        active={knowledgeView}
        onSelect={setKnowledgeView}
      />

      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Logbook keeps Hive&apos;s recent durable writeups close at hand. Scan recent briefs here, then open the full writeup without leaving the workspace.
      </Text>

      <View style={styles.metricsRow}>
        <MakoInsightCard
          label="Recent"
          value={String(visibleReports.length)}
          detail={query.trim() ? "matching this filter" : "visible in this scope"}
          style={styles.metricCard}
        />
        <MakoInsightCard
          label="Projects"
          value={String(uniqueProjects)}
          detail={reports.scope === "workspace" ? "in this workspace" : "represented here"}
          style={styles.metricCard}
        />
      </View>

      <View style={styles.metricsRow}>
        <MakoInsightCard
          label="Latest"
          value={formatRelativeTime(visibleReports[0]?.created_at)}
          detail="most recent report"
          style={styles.metricCard}
        />
        <MakoInsightCard
          label="Top tag"
          value={topTag.value}
          detail={topTag.detail}
          tone={topTag.value === "None" ? "default" : "accent"}
          style={styles.metricCard}
        />
      </View>

      <MakoKnowledgeScopeToggle
        activeScope={reports.scope}
        workspaceDirectory={workspaceDirectory}
        allLabel="All reports"
        allHint="across every workspace"
        onSelect={reports.setScope}
      />

      <TextInput
        value={query}
        onChangeText={setQuery}
        placeholder="Search titles, summaries, tags, or sources"
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

      {reports.error ? (
        <Text style={[styles.error, { color: t.error }]}>{reports.error}</Text>
      ) : null}

      {visibleReports.length === 0 ? (
        <View style={[styles.emptyCard, { borderColor: t.border }]}>
          <Text style={[styles.emptyTitle, { color: t.foreground }]}>
            {reports.reports.length === 0 && !reports.debouncedQuery
              ? "No reports yet"
              : "No matching reports"}
          </Text>
          <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
            {reports.reports.length === 0 && !reports.debouncedQuery
              ? "Hive has not written any durable briefs for this scope yet."
              : "Try a different search term or switch scopes."}
          </Text>
        </View>
      ) : (
        visibleReports.map((report) => (
          <ReportCard
            key={report.id}
            report={report}
            selected={report.id === reports.selectedReportId}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              void reports.selectReport(report.id);
            }}
          />
        ))
      )}
    </>
  );

  if (reports.isLoading && reports.reports.length === 0) {
    return (
      <View style={styles.loading}>
        <ListRowsSkeleton rows={7} />
      </View>
    );
  }

  if (isDesktop) {
    return (
      <View style={styles.desktopLayout}>
        <ScrollView
          style={styles.desktopColumn}
          contentContainerStyle={styles.content}
          refreshControl={
            <RefreshControl
              refreshing={reports.isRefreshing}
              onRefresh={() => {
                void reports.refresh();
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
          <ReportDetailPane
            isLoading={reports.isDetailLoading}
            report={reports.selectedReport}
            isPromoting={isPromoting}
            promotionState={promotionState}
            onPromote={handlePromote}
          />
        </ScrollView>
      </View>
    );
  }

  if (reports.selectedReportId) {
    return (
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        refreshControl={
          <RefreshControl
            refreshing={reports.isRefreshing}
            onRefresh={() => {
              void reports.refresh();
            }}
            tintColor={t.userMessage}
          />
        }
        showsVerticalScrollIndicator={false}
      >
        <MakoTopNav
          items={[
            { id: "recent", label: "Recent" },
            { id: "memory", label: "Memory" },
          ]}
          active={knowledgeView}
          onSelect={setKnowledgeView}
        />

        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            reports.clearSelection();
          }}
          style={styles.backButton}
        >
          <Text style={[styles.backLabel, { color: t.userMessage }]}>
            Back to logbook
          </Text>
        </Pressable>

        <ReportDetailPane
          isLoading={reports.isDetailLoading}
          report={reports.selectedReport}
          isPromoting={isPromoting}
          promotionState={promotionState}
          onPromote={handlePromote}
        />
      </ScrollView>
    );
  }

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={reports.isRefreshing}
          onRefresh={() => {
            void reports.refresh();
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
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 14,
  },
  error: {
    fontSize: 13,
    lineHeight: 18,
  },
  card: {
    marginBottom: 0,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    padding: 12,
  },
  title: {
    fontSize: 14,
    fontWeight: "600",
    lineHeight: 22,
  },
  summary: {
    marginTop: 8,
    fontSize: 13,
    lineHeight: 18,
  },
  metaRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
    marginTop: 14,
  },
  footer: {
    marginTop: 10,
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
    alignItems: "center",
  },
  date: {
    fontSize: 12,
    fontWeight: "500",
  },
  tags: {
    flex: 1,
    textAlign: "right",
    fontSize: 12,
    fontWeight: "600",
  },
  selectedLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  emptyCard: {
    marginBottom: 0,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    padding: 12,
  },
  emptyTitle: {
    fontSize: 14,
    fontWeight: "600",
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
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    padding: 12,
  },
  detailTitle: {
    fontSize: 18,
    fontWeight: "600",
    lineHeight: 24,
  },
  detailCopy: {
    marginTop: 8,
    marginBottom: 14,
    fontSize: 13,
    lineHeight: 18,
  },
  promoteButton: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    marginBottom: 10,
  },
  promoteLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  promotionMessage: {
    marginBottom: 14,
    fontSize: 12,
    lineHeight: 16,
  },
});
