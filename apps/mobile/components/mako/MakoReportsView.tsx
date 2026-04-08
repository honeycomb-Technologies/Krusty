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
import type { Report } from "@krusty/api";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { ReportDetailContent } from "../reports/ReportDetailContent";
import { GlassCard } from "../ui/GlassCard";
import { MakoInsightCard } from "./MakoInsightCard";
import { useMakoReports, type MakoReportScope } from "./hooks/useMakoReports";
import { formatProjectLabel, formatRelativeTime } from "./utils";

interface MakoReportsViewProps {
  workspaceDirectory?: string | null;
}

function matchesReportQuery(
  report: {
    title: string;
    summary: string;
    tags: string[];
    project_dir?: string;
  },
  query: string,
): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) {
    return true;
  }

  return [report.title, report.summary, report.project_dir ?? "", ...report.tags]
    .join(" ")
    .toLowerCase()
    .includes(normalizedQuery);
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

function ScopeToggle({
  activeScope,
  workspaceDirectory,
  onSelect,
}: {
  activeScope: MakoReportScope;
  workspaceDirectory?: string | null;
  onSelect: (scope: MakoReportScope) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.scopeRow}>
      {workspaceDirectory ? (
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            onSelect("workspace");
          }}
          style={[
            styles.scopePill,
            {
              backgroundColor:
                activeScope === "workspace" ? `${t.userMessage}18` : "transparent",
              borderColor:
                activeScope === "workspace" ? `${t.userMessage}44` : t.border,
            },
          ]}
        >
          <Text
            style={[
              styles.scopeLabel,
              {
                color:
                  activeScope === "workspace" ? t.userMessage : t.mutedForeground,
              },
            ]}
          >
            Current workspace
          </Text>
          <Text style={[styles.scopeHint, { color: t.mutedForeground }]}>
            {formatProjectLabel(workspaceDirectory)}
          </Text>
        </Pressable>
      ) : null}

      <Pressable
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onSelect("all");
        }}
        style={[
          styles.scopePill,
          {
            backgroundColor:
              activeScope === "all" ? `${t.userMessage}18` : "transparent",
            borderColor: activeScope === "all" ? `${t.userMessage}44` : t.border,
          },
        ]}
      >
        <Text
          style={[
            styles.scopeLabel,
            {
              color: activeScope === "all" ? t.userMessage : t.mutedForeground,
            },
          ]}
        >
          All reports
        </Text>
        <Text style={[styles.scopeHint, { color: t.mutedForeground }]}>
          across every workspace
        </Text>
      </Pressable>
    </View>
  );
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
      <GlassCard style={styles.card} elevated={selected}>
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
      </GlassCard>
    </Pressable>
  );
}

function DetailPane({
  isLoading,
  report,
}: {
  isLoading: boolean;
  report: {
    id: string;
    title: string;
    content: string;
    summary: string;
    tags: string[];
    sources: string[];
    session_id: string;
    created_at: string;
    project_dir?: string;
  } | Report | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (isLoading) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={t.userMessage} />
      </View>
    );
  }

  if (!report) {
    return (
      <GlassCard style={styles.detailCard} elevated>
        <Text style={[styles.detailTitle, { color: t.foreground }]}>
          No report selected
        </Text>
        <Text style={[styles.detailCopy, { color: t.mutedForeground }]}>
          Open any report to inspect the written brief, sources, and project context.
        </Text>
      </GlassCard>
    );
  }

  return (
    <GlassCard style={styles.detailCard} elevated>
      <Text style={[styles.detailTitle, { color: t.foreground }]}>{report.title}</Text>
      {report.summary ? (
        <Text style={[styles.detailCopy, { color: t.mutedForeground }]}>
          {report.summary}
        </Text>
      ) : null}
      <ReportDetailContent report={report} />
    </GlassCard>
  );
}

export function MakoReportsView({ workspaceDirectory }: MakoReportsViewProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const [query, setQuery] = useState("");
  const reports = useMakoReports(true, workspaceDirectory);

  const visibleReports = reports.reports.filter((report) =>
    matchesReportQuery(report, query),
  );
  const visibleReportIds = visibleReports.map((report) => report.id).join("|");
  const uniqueProjects = new Set(
    visibleReports
      .map((report) => report.project_dir)
      .filter((projectDir): projectDir is string => Boolean(projectDir)),
  ).size;
  const topTag = topTagLabel(visibleReports);

  useEffect(() => {
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
    isDesktop,
    reports.clearSelection,
    reports.selectedReportId,
    reports.selectReport,
    visibleReportIds,
  ]);

  const listContent = (
    <>
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Reports are Mako&apos;s durable writeups. This surface should make it easy to scan what has already been learned, then reopen the full brief without leaving the workspace.
      </Text>

      <View style={styles.metricsRow}>
        <MakoInsightCard
          label="Reports"
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

      <ScopeToggle
        activeScope={reports.scope}
        workspaceDirectory={workspaceDirectory}
        onSelect={reports.setScope}
      />

      <TextInput
        value={query}
        onChangeText={setQuery}
        placeholder="Search reports or tags"
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
        <GlassCard style={styles.emptyCard}>
          <Text style={[styles.emptyTitle, { color: t.foreground }]}>
            {reports.reports.length === 0 ? "No reports yet" : "No matching reports"}
          </Text>
          <Text style={[styles.emptyBody, { color: t.mutedForeground }]}>
            {reports.reports.length === 0
              ? "Mako has not written any durable briefs for this scope yet."
              : "Try a different search term or switch scopes."}
          </Text>
        </GlassCard>
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
        <ActivityIndicator color={t.userMessage} />
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
          <DetailPane
            isLoading={reports.isDetailLoading}
            report={reports.selectedReport}
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
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            reports.clearSelection();
          }}
          style={styles.backButton}
        >
          <Text style={[styles.backLabel, { color: t.userMessage }]}>
            Back to reports
          </Text>
        </Pressable>

        <DetailPane
          isLoading={reports.isDetailLoading}
          report={reports.selectedReport}
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
  scopeRow: {
    flexDirection: "row",
    gap: 12,
  },
  scopePill: {
    flex: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 18,
    paddingHorizontal: 14,
    paddingVertical: 12,
    gap: 2,
  },
  scopeLabel: {
    fontSize: 13,
    fontWeight: "700",
  },
  scopeHint: {
    fontSize: 11,
    lineHeight: 15,
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
  title: {
    fontSize: 16,
    fontWeight: "700",
    lineHeight: 22,
  },
  summary: {
    marginTop: 10,
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
  detailCopy: {
    marginTop: 10,
    marginBottom: 18,
    fontSize: 13,
    lineHeight: 18,
  },
});
