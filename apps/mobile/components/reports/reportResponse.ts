import type { ReportSummary } from "@mitsuro/api";

export function reportSummariesFromResponse(value: unknown): ReportSummary[] {
  if (!value || typeof value !== "object") return [];
  const reports = (value as { reports?: unknown }).reports;
  return Array.isArray(reports) ? reports : [];
}
