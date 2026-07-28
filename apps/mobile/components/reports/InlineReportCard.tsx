import { useEffect, useState } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { ChevronDown, ChevronRight, FileText, X } from "lucide-react-native";
import type { Report } from "@krusty/api";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { DetailPaneSkeleton } from "../ui/Skeleton";
import { ReportDetailContent } from "./ReportDetailContent";

interface InlineReportCardProps {
  reportId: string;
  defaultExpanded?: boolean;
  onDismiss?: () => void;
}

export function InlineReportCard({
  reportId,
  defaultExpanded = false,
  onDismiss,
}: InlineReportCardProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [report, setReport] = useState<Report | null>(null);
  const [isLoading, setIsLoading] = useState(defaultExpanded);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!expanded || report?.id === reportId) {
      return;
    }
    if (!client) {
      setIsLoading(false);
      setError("Report is unavailable while disconnected.");
      return;
    }

    let cancelled = false;
    setIsLoading(true);
    setError(null);
    void client
      .getReport(reportId)
      .then((nextReport) => {
        if (!cancelled) {
          setReport(nextReport);
        }
      })
      .catch((loadError: unknown) => {
        if (!cancelled) {
          setError(
            loadError instanceof Error ? loadError.message : "Failed to load report.",
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, expanded, report?.id, reportId]);

  return (
    <View
      style={[
        styles.card,
        { borderColor: t.border, backgroundColor: t.card },
      ]}
    >
      <View style={styles.header}>
        <Pressable
          onPress={() => setExpanded((value) => !value)}
          style={styles.toggle}
        >
          <FileText size={15} color={t.userMessage} />
          <View style={styles.titleBlock}>
            <Text
              style={[styles.title, { color: t.foreground }]}
              numberOfLines={1}
            >
              {report?.title ?? "Mako report"}
            </Text>
            {!expanded && report?.summary ? (
              <Text
                style={[styles.summary, { color: t.mutedForeground }]}
                numberOfLines={2}
              >
                {report.summary}
              </Text>
            ) : null}
          </View>
          {expanded ? (
            <ChevronDown size={15} color={t.mutedForeground} />
          ) : (
            <ChevronRight size={15} color={t.mutedForeground} />
          )}
        </Pressable>
        {onDismiss ? (
          <Pressable onPress={onDismiss} hitSlop={8}>
            <X size={15} color={t.mutedForeground} />
          </Pressable>
        ) : null}
      </View>

      {expanded ? (
        <View style={[styles.content, { borderTopColor: t.border }]}>
          {isLoading ? <DetailPaneSkeleton /> : null}
          {error ? <Text style={[styles.error, { color: t.error }]}>{error}</Text> : null}
          {report ? <ReportDetailContent report={report} /> : null}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 12,
    paddingVertical: 10,
    gap: 8,
  },
  toggle: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  titleBlock: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 13,
    fontWeight: "600",
  },
  summary: {
    marginTop: 2,
    fontSize: 12,
    lineHeight: 17,
  },
  content: {
    borderTopWidth: StyleSheet.hairlineWidth,
    padding: 12,
  },
  error: {
    fontSize: 12,
    lineHeight: 17,
  },
});
