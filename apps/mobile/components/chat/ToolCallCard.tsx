import { useEffect, useState } from "react";
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
import type { ToolCall } from "@krusty/api";

interface ToolCallCardProps {
  toolCall: ToolCall;
  isStreaming?: boolean;
  defaultExpanded?: boolean;
}

export function ToolCallCard({
  toolCall,
  isStreaming,
  defaultExpanded = false,
}: ToolCallCardProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [expanded, setExpanded] = useState(defaultExpanded);
  const showDetailedSurface =
    expanded ||
    Boolean(isStreaming) ||
    toolCall.status === "awaiting_approval";
  const isPolicyRedirect =
    toolCall.status === "error" &&
    Boolean(
      toolCall.output?.includes("not allowed here") &&
        toolCall.output?.includes("use the dedicated"),
    );

  useEffect(() => {
    setExpanded(defaultExpanded);
  }, [defaultExpanded, toolCall.id]);

  const StatusIcon = () => {
    switch (toolCall.status) {
      case "running":
        return <ToolGlyph name={toolCall.name} color={t.thinking} />;
      case "success":
        return <Check size={14} color={t.success} strokeWidth={2.5} />;
      case "error":
        if (isPolicyRedirect) {
          return <CornerDownRight size={14} color={t.warning} strokeWidth={2} />;
        }
        return <X size={14} color={t.error} strokeWidth={2.5} />;
      case "awaiting_approval":
        return <Clock size={14} color={t.warning} strokeWidth={2} />;
      default:
        return <ToolGlyph name={toolCall.name} color={t.mutedForeground} />;
    }
  };

  const toggle = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setExpanded(!expanded);
  };

  // Parse arguments for display
  const args = toolCall.arguments ?? {};
  const filePath = (args.file_path ??
    args.path ??
    args.pattern ??
    "") as string;
  const command = (args.command ?? "") as string;

  // Tool-specific rendering
  const renderBody = () => {
    const name = toolCall.name;

    if (toolCall.status === "error" && !expanded && !isStreaming) {
      return (
        <Text
          style={[
            styles.failedSummary,
            { color: isPolicyRedirect ? t.warning : t.error },
          ]}
          numberOfLines={2}
        >
          {toolCall.output || (isPolicyRedirect ? "Rerouted" : "Tool failed")}
        </Text>
      );
    }

    if (toolCall.delegated) {
      const delegated = toolCall.delegated;
      const statusLine = [
        delegated.outcome ?? delegated.stage,
        delegated.agentCount !== undefined
          ? `${delegated.agentCount} agent${delegated.agentCount === 1 ? "" : "s"}`
          : undefined,
        delegated.failedAgents !== undefined
          ? `${delegated.failedAgents} failed`
          : undefined,
        delegated.filesExaminedCount !== undefined
          ? `${delegated.filesExaminedCount} paths`
          : undefined,
      ]
        .filter(Boolean)
        .join(" · ");
      const summary =
        delegated.message ||
        delegated.investigationSummary ||
        delegated.humanReview ||
        toolCall.output;

      return (
        <View style={styles.delegatedBody}>
          <View style={styles.fileRow}>
            <Users size={14} color={t.mutedForeground} strokeWidth={1.5} />
            <Text
              style={[styles.filePath, { color: t.mutedForeground }]}
              numberOfLines={1}
            >
              {delegated.kind}
              {delegated.delegatedRunId ? ` · ${delegated.delegatedRunId}` : ""}
            </Text>
          </View>
          {statusLine ? (
            <Text style={[styles.countBadge, { color: t.mutedForeground }]}>
              {statusLine}
            </Text>
          ) : null}
          {summary ? (
            <Text
              style={[styles.outputText, { color: t.foreground }]}
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
                  style={[styles.agentLine, { color: t.mutedForeground }]}
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

    // Bash tool — terminal output
    if (name === "bash" || name === "Bash") {
      if (!expanded && !isStreaming) {
        return command ? (
          <Text
            style={[styles.collapsedCommand, { color: t.mutedForeground }]}
            numberOfLines={1}
          >
            $ {command}
          </Text>
        ) : null;
      }
      return (
        <BashOutput
          command={command || undefined}
          output={toolCall.output ?? ""}
        />
      );
    }

    // Edit/Write — show file path and diff summary
    if (
      name === "edit" ||
      name === "write" ||
      name === "multiedit" ||
      name === "Edit" ||
      name === "Write" ||
      name === "MultiEdit"
    ) {
      const oldStr = (args.old_string ?? "") as string;
      const newStr = (args.new_string ?? args.content ?? "") as string;
      const addedLines = newStr.split("\n").length;
      const removedLines = oldStr ? oldStr.split("\n").length : 0;

      return (
        <View style={styles.diffSummary}>
          {filePath ? (
            <Text style={[styles.filePath, { color: t.mutedForeground }]}>
              {filePath}
            </Text>
          ) : null}
          <View style={styles.diffStats}>
            {addedLines > 0 && (
              <Text style={styles.addedText}>+{addedLines}</Text>
            )}
            {removedLines > 0 && (
              <Text style={styles.removedText}>-{removedLines}</Text>
            )}
          </View>
          {expanded && toolCall.output && (
            <Text
              style={[styles.outputText, { color: t.foreground }]}
              selectable
              numberOfLines={30}
            >
              {toolCall.output.slice(0, 3000)}
            </Text>
          )}
        </View>
      );
    }

    // Read — file icon + path
    if (name === "read" || name === "Read") {
      return filePath ? (
        <View style={styles.fileRow}>
          <FileText size={14} color={t.mutedForeground} strokeWidth={1.5} />
          <Text
            style={[styles.filePath, { color: t.mutedForeground }]}
            numberOfLines={1}
          >
            {filePath}
          </Text>
        </View>
      ) : null;
    }

    // Glob/Grep — expandable results
    if (
      name === "glob" ||
      name === "grep" ||
      name === "Glob" ||
      name === "Grep"
    ) {
      const resultLines = (toolCall.output ?? "").split("\n").filter(Boolean);
      return (
        <View>
          <View style={styles.fileRow}>
            {name.toLowerCase() === "grep" ? (
              <Search size={14} color={t.mutedForeground} strokeWidth={1.5} />
            ) : (
              <FolderTree
                size={14}
                color={t.mutedForeground}
                strokeWidth={1.5}
              />
            )}
            <Text style={[styles.filePath, { color: t.mutedForeground }]}>
              {filePath || "results"}
            </Text>
            <Text style={[styles.countBadge, { color: t.mutedForeground }]}>
              ({resultLines.length})
            </Text>
          </View>
          {expanded && (
            <Text
              style={[styles.outputText, { color: t.foreground }]}
              selectable
              numberOfLines={50}
            >
              {resultLines.slice(0, 50).join("\n")}
            </Text>
          )}
        </View>
      );
    }

    // Default — just show output if expanded
    if (expanded && toolCall.output) {
      return (
        <Text
          style={[styles.outputText, { color: t.foreground }]}
          selectable
          numberOfLines={30}
        >
          {toolCall.output.slice(0, 3000)}
        </Text>
      );
    }

    return null;
  };

  return (
    <Pressable
      onPress={toggle}
      style={[
        styles.card,
        showDetailedSurface
          ? [
              styles.detailedCard,
              { borderColor: t.border, backgroundColor: t.card },
            ]
          : styles.compactCard,
      ]}
    >
      <View style={styles.header}>
        <StatusIcon />
        <Text
          style={[styles.toolName, { color: t.foreground }]}
          numberOfLines={1}
        >
          {formatToolLabel(toolCall.name)}
        </Text>
        {isPolicyRedirect ? (
          <Text style={[styles.liveLabel, { color: t.warning }]}>Redirected</Text>
        ) : null}
        {isStreaming ? (
          <Text style={[styles.liveLabel, { color: t.thinking }]}>Live</Text>
        ) : null}
        {toolCall.output &&
          (expanded ? (
            <ChevronDown size={14} color={t.mutedForeground} />
          ) : (
            <ChevronRight size={14} color={t.mutedForeground} />
          ))}
      </View>
      {renderBody()}
    </Pressable>
  );
}

function ToolGlyph({ name, color }: { name: string; color: string }) {
  const normalized = name.toLowerCase();
  const iconProps = { size: 14, color, strokeWidth: 1.8 } as const;

  if (normalized === "bash") return <Terminal {...iconProps} />;
  if (["edit", "write", "multiedit"].includes(normalized)) {
    return <FilePenLine {...iconProps} />;
  }
  if (normalized === "read") return <FileText {...iconProps} />;
  if (["grep", "search"].includes(normalized)) {
    return <Search {...iconProps} />;
  }
  if (["glob", "ls", "list", "list_files"].includes(normalized)) {
    return <FolderTree {...iconProps} />;
  }
  if (["agent", "explore", "plan", "verify", "build"].includes(normalized)) {
    return <Users {...iconProps} />;
  }
  return <Wrench {...iconProps} />;
}

function formatToolLabel(name: string): string {
  return name
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

const styles = StyleSheet.create({
  card: {
    marginVertical: 3,
  },
  compactCard: {
    paddingVertical: 5,
    paddingHorizontal: 2,
  },
  detailedCard: {
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 10,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  toolName: {
    flex: 1,
    fontSize: 13,
    fontWeight: "500",
    fontFamily: "Courier",
  },
  liveLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  delegatedBody: { marginTop: 6, gap: 4 },
  agentList: { marginTop: 6, gap: 3 },
  agentLine: { fontSize: 11, fontFamily: "Courier", lineHeight: 15 },
  diffSummary: { marginTop: 6, gap: 4 },
  diffStats: { flexDirection: "row", gap: 10 },
  addedText: { color: "#22c55e", fontSize: 12, fontFamily: "Courier" },
  removedText: { color: "#ef4444", fontSize: 12, fontFamily: "Courier" },
  filePath: { fontSize: 12, fontFamily: "Courier" },
  fileRow: { flexDirection: "row", alignItems: "center", gap: 6, marginTop: 4 },
  countBadge: { fontSize: 11 },
  outputText: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
    marginTop: 6,
    opacity: 0.85,
  },
  failedSummary: {
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 15,
    marginTop: 5,
  },
  collapsedCommand: {
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 15,
    marginTop: 5,
  },
});
