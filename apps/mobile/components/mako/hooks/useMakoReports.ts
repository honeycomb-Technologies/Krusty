import { useCallback, useEffect, useState } from "react";
import { useConnection } from "../../../hooks/useConnection";
import type { Report, ReportSummary } from "@krusty/api";
import type { MakoKnowledgeScope } from "../types";

export function useMakoReports(
  enabled: boolean,
  workspaceDirectory?: string | null,
  query?: string,
) {
  const { client, isConnected } = useConnection();
  const [scope, setScope] = useState<MakoKnowledgeScope>(
    workspaceDirectory ? "workspace" : "all",
  );
  const [reports, setReports] = useState<ReportSummary[]>([]);
  const [selectedReportId, setSelectedReportId] = useState<string | null>(null);
  const [selectedReport, setSelectedReport] = useState<Report | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isDetailLoading, setIsDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [debouncedQuery, setDebouncedQuery] = useState(query?.trim() ?? "");

  useEffect(() => {
    const timeout = setTimeout(() => {
      setDebouncedQuery(query?.trim() ?? "");
    }, 180);

    return () => {
      clearTimeout(timeout);
    };
  }, [query]);

  useEffect(() => {
    if (!workspaceDirectory && scope === "workspace") {
      setScope("all");
    }
  }, [scope, workspaceDirectory]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let cancelled = false;

    const loadReports = async (refreshing: boolean) => {
      if (!client || !isConnected) {
        if (!cancelled) {
          setReports([]);
          setSelectedReportId(null);
          setSelectedReport(null);
          setIsLoading(false);
          setIsRefreshing(false);
        }
        return;
      }

      if (refreshing) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }
      setError(null);

      try {
        const response = await client.getReports(
          {
            projectDir: scope === "workspace" ? workspaceDirectory ?? undefined : undefined,
            query: debouncedQuery || undefined,
          },
        );

        if (cancelled) {
          return;
        }

        setReports(response.reports);
      } catch (loadError) {
        if (cancelled) {
          return;
        }

        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to load reports.",
        );
        setReports([]);
        setSelectedReportId(null);
        setSelectedReport(null);
      } finally {
        if (!cancelled) {
          setIsLoading(false);
          setIsRefreshing(false);
        }
      }
    };

    void loadReports(false);

    return () => {
      cancelled = true;
    };
  }, [client, debouncedQuery, enabled, isConnected, scope, workspaceDirectory]);

  useEffect(() => {
    if (selectedReportId && !reports.some((report) => report.id === selectedReportId)) {
      setSelectedReportId(null);
      setSelectedReport(null);
    }
  }, [reports, selectedReportId]);

  const refresh = useCallback(async () => {
    if (!client || !isConnected) {
      setReports([]);
      setSelectedReportId(null);
      setSelectedReport(null);
      setIsLoading(false);
      setIsRefreshing(false);
      return;
    }

    setIsRefreshing(true);
    setError(null);
    try {
      const response = await client.getReports(
        {
          projectDir: scope === "workspace" ? workspaceDirectory ?? undefined : undefined,
          query: debouncedQuery || undefined,
        },
      );
      setReports(response.reports);
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to refresh reports.",
      );
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, debouncedQuery, isConnected, scope, workspaceDirectory]);

  const selectReport = useCallback(async (reportId: string) => {
    if (!client || !isConnected) {
      return;
    }

    if (selectedReport?.id === reportId) {
      setSelectedReportId(reportId);
      return;
    }

    setSelectedReportId(reportId);
    setSelectedReport(null);
    setIsDetailLoading(true);
    setError(null);

    try {
      const report = await client.getReport(reportId);
      setSelectedReport(report);
    } catch (detailError) {
      setSelectedReportId(null);
      setSelectedReport(null);
      setError(
        detailError instanceof Error
          ? detailError.message
          : "Failed to open report.",
      );
    } finally {
      setIsDetailLoading(false);
    }
  }, [client, isConnected, selectedReport?.id]);

  const clearSelection = useCallback(() => {
    setSelectedReportId(null);
    setSelectedReport(null);
    setIsDetailLoading(false);
  }, []);

  return {
    scope,
    setScope,
    reports,
    selectedReportId,
    selectedReport,
    isLoading,
    isRefreshing,
    isDetailLoading,
    error,
    debouncedQuery,
    refresh,
    selectReport,
    clearSelection,
  };
}
