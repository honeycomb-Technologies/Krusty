import { memo, useEffect, useMemo, useState } from "react";
import { View, Text, Pressable, StyleSheet } from "react-native";
import {
  Check,
  X,
  Clock,
  ChevronDown,
  ChevronRight,
  FileText,
  FilePenLine,
  Search,
  FolderTree,
  Terminal,
  Users,
  Wrench,
  CornerDownRight,
} from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { BashOutput } from "./BashOutput";
import { ToolDiffViewer } from "./ToolDiffViewer";
import { InlineReportCard } from "../reports/InlineReportCard";
import { ToolDiffPeek } from "./ToolDiffPeek";
import {
  buildReadFilePresentation,
  buildToolDiffPeekModel,
} from "./toolDiffModel";
import {
  presentTool,
  type ToolPresentation,
} from "./toolPresentation";
import type { ToolCall } from "@krusty/api";

interface ToolCallCardProps {
  toolCall: ToolCall;
  isStreaming?: boolean;
  defaultExpanded?: boolean;
  /** Nested chips inside an exploration cluster stay extra quiet. */
  compact?: boolean;
}

export const ToolCallCard = memo(function ToolCallCard({
  toolCall,
  isStreaming,
  defaultExpanded,
  compact = false,
}: ToolCallCardProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const presentation = useMemo(
    () => presentTool(toolCall, { isStreaming }),
    [
      isStreaming,
      toolCall.arguments,
      toolCall.delegated,
      toolCall.delegatedRunId,
      toolCall.description,
      toolCall.id,
      toolCall.name,
      toolCall.output,
      toolCall.status,
    ],
  );
  const diffPeek = useMemo(
    () => presentation.diff && presentation.showDiffPeek
      ? buildToolDiffPeekModel(presentation.diff, presentation.peekDiffRows)
      : null,
    [presentation],
  );
  const [expanded, setExpanded] = useState(
    defaultExpanded ?? presentation.defaultExpanded,
  );

  // Sticky open/close state: only re-seed when the tool identity changes.
  useEffect(() => {
    setExpanded(defaultExpanded ?? presentation.defaultExpanded);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- preserve user expand across status mutations
  }, [toolCall.id]);

  const reportId =
    toolCall.name.toLowerCase() === "report"
      ? extractReportId(toolCall)
      : null;
  if (reportId) {
    return (
      <InlineReportCard reportId={reportId} defaultExpanded={defaultExpanded} />
    );
  }

  if (presentation.family === "hidden") {
    return null;
  }

  const toggle = () => {
    if (!presentation.canExpand && !presentation.showDiffPeek) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setExpanded((current) => !current);
  };

  const showElevated =
    !compact &&
    (expanded || toolCall.status === "awaiting_approval");

  const titleColor =
    toolCall.status === "error"
      ? presentation.isPolicyRedirect
        ? t.warning
        : t.error
      : toolCall.status === "partial"
        ? t.warning
      : toolCall.status === "running"
        ? t.foreground
        : t.mutedForeground;

  const canToggle =
    presentation.canExpand || presentation.showDiffPeek || Boolean(toolCall.output);

  return (
    <View
      style={[
        styles.card,
        showElevated
          ? [
              styles.detailedCard,
              { borderColor: t.border, backgroundColor: t.card },
            ]
          : styles.compactCard,
        compact && styles.clusterCard,
      ]}
    >
      <Pressable
        onPress={toggle}
        disabled={!canToggle}
        style={styles.header}
        accessibilityRole={canToggle ? "button" : undefined}
      >
        <StatusIcon
          toolCall={toolCall}
          presentation={presentation}
          colors={t}
        />
        <Text style={[styles.toolName, { color: titleColor }]} numberOfLines={1}>
          {presentation.label}
        </Text>
        <Text
          style={[styles.summary, { color: t.mutedForeground }]}
          numberOfLines={1}
        >
          {presentation.summary}
        </Text>
        {presentation.meta ? (
          <Text style={[styles.meta, metaColor(presentation, t)]} numberOfLines={1}>
            {presentation.meta}
          </Text>
        ) : null}
        {presentation.isPolicyRedirect ? (
          <Text style={[styles.liveLabel, { color: t.warning }]}>Redirected</Text>
        ) : null}
        {canToggle ? (
          expanded ? (
            <ChevronDown size={14} color={t.mutedForeground} />
          ) : (
            <ChevronRight size={14} color={t.mutedForeground} />
          )
        ) : null}
      </Pressable>
      {renderBody({
        toolCall,
        presentation,
        expanded,
        isStreaming: Boolean(isStreaming),
        colors: t,
        diffPeek,
      })}
    </View>
  );
}, (previous, next) =>
  previous.isStreaming === next.isStreaming &&
  previous.defaultExpanded === next.defaultExpanded &&
  previous.compact === next.compact &&
  sameToolCallPresentation(previous.toolCall, next.toolCall),
);

function renderBody({
  toolCall,
  presentation,
  expanded,
  isStreaming,
  colors,
  diffPeek,
}: {
  toolCall: ToolCall;
  presentation: ToolPresentation;
  expanded: boolean;
  isStreaming: boolean;
  colors: ReturnType<typeof useThemeContext>["theme"]["colors"];
  diffPeek: ReturnType<typeof buildToolDiffPeekModel> | null;
}) {
  if (presentation.family === "bash") {
    const command =
      typeof toolCall.arguments?.command === "string"
        ? toolCall.arguments.command
        : undefined;
    if (expanded) {
      return (
        <BashOutput
          command={command}
          output={toolCall.output ?? ""}
          mode="log"
        />
      );
    }
    if (presentation.showRunningTail || (toolCall.status === "error" && toolCall.output)) {
      return (
        <BashOutput
          command={undefined}
          output={toolCall.output ?? ""}
          mode="tail"
          maxTailLines={toolCall.status === "error" ? 3 : 2}
        />
      );
    }
    return null;
  }

  if (presentation.family === "edit" || presentation.family === "write") {
    const diff = presentation.diff;
    if (!diff) {
      if (!expanded || !toolCall.output) return null;
      return (
        <Text
          style={[styles.outputText, { color: colors.foreground }]}
          selectable
          numberOfLines={30}
        >
          {toolCall.output.slice(0, 3000)}
        </Text>
      );
    }

    if (expanded) {
      return (
        <View style={styles.diffBody}>
          {diff.summary ? (
            <Text style={[styles.diffMessage, { color: colors.mutedForeground }]}>
              {diff.summary}
            </Text>
          ) : null}
          <ToolDiffViewer presentation={diff} />
        </View>
      );
    }

    if (presentation.showDiffPeek) {
      const peekRows = diffPeek?.rows ?? [];
      const remaining = Math.max(
        0,
        (diffPeek?.changedRowCount ?? 0) - peekRows.length,
      );
      return (
        <View style={styles.diffBody}>
          {diff.summary ? (
            <Text style={[styles.diffMessage, { color: colors.mutedForeground }]}>
              {diff.summary}
            </Text>
          ) : null}
          <ToolDiffPeek rows={peekRows} />
          {remaining > 0 ? (
            <Text style={[styles.moreLines, { color: colors.mutedForeground }]}>
              Show full diff · {remaining} more changed lines
            </Text>
          ) : (
            <Text style={[styles.moreLines, { color: colors.mutedForeground }]}>
              Show full diff
            </Text>
          )}
        </View>
      );
    }

    return null;
  }

  if (toolCall.delegated || presentation.family === "delegated") {
    const delegated = toolCall.delegated;
    if (!delegated) {
      if (!expanded || !toolCall.output) return null;
      return (
        <Text
          style={[styles.outputText, { color: colors.foreground }]}
          selectable
          numberOfLines={30}
        >
          {toolCall.output.slice(0, 3000)}
        </Text>
      );
    }

    const summary =
      delegated.message ||
      delegated.investigationSummary ||
      delegated.humanReview ||
      toolCall.output;

    return (
      <View style={styles.delegatedBody}>
        {summary ? (
          <Text
            style={[styles.outputText, { color: colors.foreground }]}
            selectable
            numberOfLines={expanded ? 30 : 3}
          >
            {summary.slice(0, expanded ? 3000 : 600)}
          </Text>
        ) : null}
        {expanded && delegated.agents.length > 0 ? (
          <View style={styles.agentList}>
            {delegated.agents.slice(0, 8).map((agent) => (
              <Text
                key={agent.taskId}
                style={[styles.agentLine, { color: colors.mutedForeground }]}
                numberOfLines={2}
              >
                {agent.status} · {agent.name}
                {agent.currentAction ? ` — ${agent.currentAction}` : ""}
              </Text>
            ))}
          </View>
        ) : null}
      </View>
    );
  }

  if (presentation.family === "read") {
    if (!expanded) return null;
    const readPresentation = buildReadFilePresentation(toolCall);
    if (readPresentation) {
      return (
        <View style={styles.diffBody}>
          <ToolDiffViewer presentation={readPresentation} />
        </View>
      );
    }
    if (!toolCall.output) return null;
    return (
      <Text
        style={[styles.outputText, { color: colors.foreground }]}
        selectable
        numberOfLines={40}
      >
        {toolCall.output.slice(0, 4000)}
      </Text>
    );
  }

  if (presentation.family === "search" || presentation.family === "explore") {
    if (!expanded || !toolCall.output) return null;
    const resultLines = toolCall.output.split("\n").filter(Boolean);
    return (
      <Text
        style={[styles.outputText, { color: colors.foreground }]}
        selectable
        numberOfLines={50}
      >
        {resultLines.slice(0, 50).join("\n")}
      </Text>
    );
  }

  if (toolCall.status === "error" && !expanded && !isStreaming) {
    return (
      <Text
        style={[
          styles.failedSummary,
          {
            color: presentation.isPolicyRedirect ? colors.warning : colors.error,
          },
        ]}
        numberOfLines={2}
      >
        {toolCall.output || (presentation.isPolicyRedirect ? "Rerouted" : "Tool failed")}
      </Text>
    );
  }

  if (expanded && toolCall.output) {
    return (
      <Text
        style={[styles.outputText, { color: colors.foreground }]}
        selectable
        numberOfLines={30}
      >
        {toolCall.output.slice(0, 3000)}
      </Text>
    );
  }

  return null;
}

/**
 * Complete semantic comparison for memoizing settled cards. Large output
 * strings are compared directly, avoiding a duplicate revision string while
 * still invalidating equal-sized content changes.
 */
export function sameToolCallPresentation(left: ToolCall, right: ToolCall): boolean {
  return left === right || (
    left.id === right.id &&
    left.name === right.name &&
    left.description === right.description &&
    left.status === right.status &&
    left.output === right.output &&
    left.delegatedRunId === right.delegatedRunId &&
    left.arguments === right.arguments &&
    left.delegated === right.delegated
  );
}

function StatusIcon({
  toolCall,
  presentation,
  colors,
}: {
  toolCall: ToolCall;
  presentation: ToolPresentation;
  colors: ReturnType<typeof useThemeContext>["theme"]["colors"];
}) {
  switch (toolCall.status) {
    case "running":
      return <ToolGlyph family={presentation.family} name={toolCall.name} color={colors.thinking} />;
    case "success":
      return <Check size={14} color={colors.success} strokeWidth={2.5} />;
    case "error":
      if (presentation.isPolicyRedirect) {
        return <CornerDownRight size={14} color={colors.warning} strokeWidth={2} />;
      }
      return <X size={14} color={colors.error} strokeWidth={2.5} />;
    case "partial":
      return (
        <ToolGlyph
          family={presentation.family}
          name={toolCall.name}
          color={colors.warning}
        />
      );
    case "awaiting_approval":
      return <Clock size={14} color={colors.warning} strokeWidth={2} />;
    default:
      return (
        <ToolGlyph
          family={presentation.family}
          name={toolCall.name}
          color={colors.mutedForeground}
        />
      );
  }
}

function ToolGlyph({
  family,
  name,
  color,
}: {
  family: ToolPresentation["family"];
  name: string;
  color: string;
}) {
  const normalized = name.toLowerCase();
  const iconProps = { size: 14, color, strokeWidth: 1.8 } as const;

  if (family === "bash" || normalized === "bash") return <Terminal {...iconProps} />;
  if (family === "edit" || family === "write") return <FilePenLine {...iconProps} />;
  if (family === "read" || normalized === "read") return <FileText {...iconProps} />;
  if (family === "search" || ["grep", "search"].includes(normalized)) {
    return <Search {...iconProps} />;
  }
  if (["glob", "ls", "list", "list_files"].includes(normalized)) {
    return <FolderTree {...iconProps} />;
  }
  if (family === "delegated") return <Users {...iconProps} />;
  return <Wrench {...iconProps} />;
}

function metaColor(
  presentation: ToolPresentation,
  colors: ReturnType<typeof useThemeContext>["theme"]["colors"],
) {
  if (presentation.family === "edit" || presentation.family === "write") {
    return { color: colors.foreground };
  }
  if (presentation.isPolicyRedirect) return { color: colors.warning };
  if (presentation.family === "bash" && presentation.meta?.includes("exit")) {
    return { color: colors.error };
  }
  return { color: colors.mutedForeground };
}

function extractReportId(toolCall: ToolCall): string | null {
  const argumentId = toolCall.arguments?.report_id;
  if (typeof argumentId === "string" && argumentId.trim()) {
    return argumentId;
  }

  if (!toolCall.output) {
    return null;
  }
  try {
    const parsed = JSON.parse(toolCall.output) as Record<string, unknown>;
    const directId = parsed.report_id;
    if (typeof directId === "string" && directId.trim()) {
      return directId;
    }
    const data =
      parsed.data && typeof parsed.data === "object" && !Array.isArray(parsed.data)
        ? (parsed.data as Record<string, unknown>)
        : null;
    return typeof data?.report_id === "string" && data.report_id.trim()
      ? data.report_id
      : null;
  } catch {
    return null;
  }
}


const styles = StyleSheet.create({
  card: {
    marginVertical: 2,
  },
  compactCard: {
    paddingVertical: 4,
    paddingHorizontal: 0,
  },
  detailedCard: {
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 10,
  },
  clusterCard: {
    marginVertical: 1,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  toolName: {
    fontSize: 12,
    fontWeight: "600",
    fontFamily: "Courier",
    flexShrink: 0,
  },
  summary: {
    flex: 1,
    fontSize: 12,
    fontFamily: "Courier",
  },
  meta: {
    fontSize: 11,
    fontFamily: "Courier",
    flexShrink: 0,
  },
  liveLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  delegatedBody: { marginTop: 6, gap: 4 },
  agentList: { marginTop: 6, gap: 3 },
  agentLine: { fontSize: 11, fontFamily: "Courier", lineHeight: 15 },
  diffBody: { marginTop: 8, gap: 6 },
  diffMessage: { fontSize: 11, lineHeight: 15 },
  moreLines: {
    fontSize: 11,
    fontFamily: "Courier",
    marginTop: 2,
  },
  outputText: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
    marginTop: 6,
    opacity: 0.9,
  },
  failedSummary: {
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 15,
    marginTop: 5,
  },
});
