import type { ToolCall } from "@mitsuro/api";
import {
  buildToolDiffPresentation,
  isDiffTool,
  type ToolDiffPresentation,
} from "./toolDiffModel";

export type ToolFamily =
  | "bash"
  | "edit"
  | "write"
  | "read"
  | "search"
  | "explore"
  | "delegated"
  | "status"
  | "hidden"
  | "question"
  | "plan_confirm";

export type ToolDensity = "hidden" | "chip" | "row" | "peek" | "card";

export type ToolSurface = "quiet" | "elevated";

export interface ToolPresentation {
  family: ToolFamily;
  density: ToolDensity;
  surface: ToolSurface;
  label: string;
  summary: string;
  meta?: string;
  isPolicyRedirect: boolean;
  canExpand: boolean;
  /** Full body open by default (rare; mostly errors/approvals). */
  defaultExpanded: boolean;
  /** Show a short live/error tail without elevating the whole widget. */
  showRunningTail: boolean;
  /** Completed mutations should reveal a short diff preview. */
  showDiffPeek: boolean;
  peekDiffRows: number;
  diff: ToolDiffPresentation | null;
}

const HIDDEN_TOOL_NAMES = new Set([
  "enter_plan_mode",
  "set_work_mode",
  "set_workspace_context",
  "task_start",
  "task_complete",
  "add_subtask",
  "set_dependency",
  "todowrite",
  "processes",
]);

const SEARCH_TOOL_NAMES = new Set([
  "glob",
  "grep",
  "ls",
  "list",
  "list_files",
  "search",
]);

const EXPLORATION_TOOL_NAMES = new Set([
  ...SEARCH_TOOL_NAMES,
  "read",
]);

const DIFF_PEEK_ROWS = 12;
const BASH_TAIL_LINES = 2;

export function isHiddenToolName(name: string): boolean {
  return HIDDEN_TOOL_NAMES.has(name);
}

export function isExplorationToolName(name: string): boolean {
  return EXPLORATION_TOOL_NAMES.has(name.toLowerCase());
}

export function isSearchToolName(name: string): boolean {
  return SEARCH_TOOL_NAMES.has(name.toLowerCase());
}

export function presentTool(
  toolCall: ToolCall,
  options: { isStreaming?: boolean } = {},
): ToolPresentation {
  const isStreaming = Boolean(options.isStreaming);
  const name = toolCall.name;
  const normalized = name.toLowerCase();
  const args = toolCall.arguments ?? {};
  const isPolicyRedirect = isPolicyRedirectOutput(toolCall);
  const family = classifyToolFamily(toolCall);
  const diff = isDiffTool(name) ? buildToolDiffPresentation(toolCall) : null;
  const summary = buildSummary(family, toolCall, args, diff);
  const meta = buildMeta(family, toolCall, diff);
  const label = buildLabel(family, name, args, toolCall.delegated?.name);
  const canExpand = canExpandTool(family, toolCall, diff);
  // Keep completion presentation sticky: no sudden peek/tail growth that reflows siblings.
  // Users can still expand intentionally for full diffs/logs.
  const showDiffPeek = false;
  const showRunningTail = false;
  const defaultExpanded =
    toolCall.status === "awaiting_approval" ||
    (toolCall.status === "error" &&
      !isPolicyRedirect &&
      family !== "bash" &&
      family !== "edit" &&
      family !== "write" &&
      family !== "read" &&
      family !== "search" &&
      family !== "explore");

  return {
    family,
    density: densityForFamily(family, toolCall, showDiffPeek),
    surface: surfaceForFamily(family, toolCall, showDiffPeek, defaultExpanded),
    label,
    summary,
    meta,
    isPolicyRedirect,
    canExpand,
    defaultExpanded,
    showRunningTail,
    showDiffPeek,
    peekDiffRows: DIFF_PEEK_ROWS,
    diff,
  };
}

export function shouldExpandToolByPolicy(
  toolCall: ToolCall,
  isStreaming = false,
): boolean {
  return presentTool(toolCall, { isStreaming }).defaultExpanded;
}

export function bashTail(output: string | undefined, maxLines = BASH_TAIL_LINES): string {
  if (!output) return "";
  const lines = output
    .replace(/\s+$/u, "")
    .split("\n")
    .filter((line, index, all) => line.length > 0 || index === all.length - 1);
  if (lines.length <= maxLines) return lines.join("\n");
  return lines.slice(-maxLines).join("\n");
}

export function shortPath(path: string, max = 42): string {
  const cleaned = path.trim();
  if (!cleaned) return "";
  if (cleaned.length <= max) return cleaned;
  const parts = cleaned.split("/").filter(Boolean);
  if (parts.length >= 2) {
    const tail = parts.slice(-2).join("/");
    if (tail.length <= max) return `…/${tail}`;
  }
  return `…${cleaned.slice(-(max - 1))}`;
}

export function formatToolLabel(name: string): string {
  return name
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function resultLineCount(output?: string): number {
  if (!output) return 0;
  return output
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean).length;
}

function classifyToolFamily(toolCall: ToolCall): ToolFamily {
  const name = toolCall.name;
  const normalized = name.toLowerCase();

  if (HIDDEN_TOOL_NAMES.has(name)) return "hidden";
  if (name === "AskUserQuestion") return "question";
  if (name === "PlanConfirm") return "plan_confirm";
  if (toolCall.delegated || isDelegatedName(normalized, toolCall.arguments)) {
    return "delegated";
  }
  if (normalized === "bash") return "bash";
  if (normalized === "read") return "read";
  if (isSearchToolName(normalized)) return "search";
  if (normalized === "write") return "write";
  if (
    normalized === "edit" ||
    normalized === "multiedit" ||
    normalized === "multi_edit" ||
    normalized === "apply_patch" ||
    normalized === "patch"
  ) {
    return "edit";
  }
  if (isExplorationToolName(normalized)) return "explore";
  return "status";
}

function isDelegatedName(
  normalized: string,
  args?: Record<string, unknown>,
): boolean {
  if (
    normalized === "explore" ||
    normalized === "plan" ||
    normalized === "verify" ||
    normalized === "build"
  ) {
    return true;
  }
  if (normalized === "agent") {
    // The current API is name + instructions + capabilities. agent_type is a
    // compatibility label, not the discriminator for whether this is a child.
    return true;
  }
  return false;
}

function densityForFamily(
  family: ToolFamily,
  _toolCall: ToolCall,
  _showDiffPeek: boolean,
): ToolDensity {
  if (family === "hidden") return "hidden";
  if (family === "edit" || family === "write" || family === "bash" || family === "delegated") {
    return "row";
  }
  if (family === "read" || family === "search" || family === "explore") {
    return "chip";
  }
  return "chip";
}

function surfaceForFamily(
  _family: ToolFamily,
  toolCall: ToolCall,
  _showDiffPeek: boolean,
  defaultExpanded: boolean,
): ToolSurface {
  // Sticky quiet rows; elevate only for interactive approval surfaces.
  if (toolCall.status === "awaiting_approval" || defaultExpanded) return "elevated";
  return "quiet";
}

function buildLabel(
  family: ToolFamily,
  name: string,
  args: Record<string, unknown>,
  delegatedName?: string,
): string {
  if (family === "bash") return "bash";
  if (family === "edit") {
    if (name.toLowerCase() === "write") return "write";
    if (name.toLowerCase() === "apply_patch" || name.toLowerCase() === "patch") {
      return "patch";
    }
    if (
      name.toLowerCase() === "multiedit" ||
      name.toLowerCase() === "multi_edit"
    ) {
      return "multi edit";
    }
    return "edit";
  }
  if (family === "write") return "write";
  if (family === "read") return "read";
  if (family === "search") {
    const normalized = name.toLowerCase();
    if (normalized === "list_files") return "list";
    return normalized;
  }
  if (family === "delegated") {
    if (name === "agent") {
      return firstString(args.name, delegatedName, args.agent_type) || "agent";
    }
    return name.toLowerCase();
  }
  return formatToolLabel(name);
}

function buildSummary(
  family: ToolFamily,
  toolCall: ToolCall,
  args: Record<string, unknown>,
  diff: ToolDiffPresentation | null,
): string {
  if (family === "bash") {
    return firstString(args.command) || toolCall.output?.split("\n")[0] || "command";
  }

  if (family === "edit" || family === "write") {
    return (
      shortPath(
        firstString(args.file_path, args.path, diff?.filePath) || "file",
      ) || "file"
    );
  }

  if (family === "read") {
    return shortPath(firstString(args.file_path, args.path) || "file") || "file";
  }

  if (family === "search") {
    return (
      firstString(args.pattern, args.path, args.query) ||
      toolCall.output?.split("\n")[0] ||
      "results"
    );
  }

  if (family === "delegated") {
    const delegated = toolCall.delegated;
    return (
      delegated?.message ||
      delegated?.investigationSummary ||
      delegated?.humanReview ||
      firstString(args.instructions) ||
      firstString(args.prompt) ||
      toolCall.output ||
      delegated?.kind ||
      "delegated work"
    );
  }

  if (toolCall.status === "error") {
    return toolCall.output || "Tool failed";
  }

  return (
    firstString(args.action, args.query, args.path, args.pattern) ||
    toolCall.output?.split("\n")[0] ||
    formatToolLabel(toolCall.name)
  );
}

function buildMeta(
  family: ToolFamily,
  toolCall: ToolCall,
  diff: ToolDiffPresentation | null,
): string | undefined {
  if (family === "edit" || family === "write") {
    if (!diff) return toolCall.status === "running" ? "writing" : undefined;
    const parts: string[] = [];
    if (diff.additions > 0) parts.push(`+${diff.additions}`);
    if (diff.deletions > 0) parts.push(`-${diff.deletions}`);
    return parts.join(" ") || undefined;
  }

  if (family === "search" || family === "read") {
    const count = resultLineCount(toolCall.output);
    if (count > 0) return `${count}`;
    if (toolCall.status === "running") return "…";
    return undefined;
  }

  if (family === "bash") {
    if (toolCall.status === "error") {
      return extractExitLabel(toolCall.output) || "failed";
    }
    if (toolCall.status === "success") {
      return extractExitLabel(toolCall.output);
    }
    return undefined;
  }

  if (family === "delegated") {
    const delegated = toolCall.delegated;
    const rawCapabilities = Array.isArray(toolCall.arguments?.capabilities)
      ? toolCall.arguments.capabilities
      : toolCall.delegated?.capabilities ?? [];
    const capabilities = rawCapabilities
      ? rawCapabilities.filter(
          (value): value is string => typeof value === "string" && value.trim().length > 0,
        )
      : [];
    const capabilityLabel = capabilities.length > 0
      ? capabilities.map((capability) => capability.toLowerCase()).join(" + ")
      : undefined;
    const delegatedStateLabel = delegated?.stage === "degraded"
      || delegated?.stage === "cancelled"
      ? delegated.stage
      : delegated?.groupState ?? delegated?.outcome ?? delegated?.stage;
    return [
      capabilityLabel,
      delegatedStateLabel,
      delegated?.agentCount !== undefined
        ? `${delegated.agentCount} agent${delegated.agentCount === 1 ? "" : "s"}`
        : undefined,
      delegated?.activeTargets ? `${delegated.activeTargets} running` : undefined,
      delegated?.pendingTargets ? `${delegated.pendingTargets} queued` : undefined,
      delegated?.completedTargets ? `${delegated.completedTargets} settled` : undefined,
      delegated?.degradedAgents ? `${delegated.degradedAgents} degraded` : undefined,
      delegated?.cancelledAgents ? `${delegated.cancelledAgents} cancelled` : undefined,
      delegated?.failedAgents ? `${delegated.failedAgents} failed` : undefined,
      delegated?.filesExaminedCount !== undefined
        ? `${delegated.filesExaminedCount} paths`
        : undefined,
    ]
      .filter(Boolean)
      .join(" · ") || undefined;
  }

  return undefined;
}

function canExpandTool(
  family: ToolFamily,
  toolCall: ToolCall,
  diff: ToolDiffPresentation | null,
): boolean {
  if (family === "hidden") return false;
  if (family === "edit" || family === "write") {
    return Boolean(diff) || Boolean(toolCall.output);
  }
  if (family === "bash") {
    return Boolean(toolCall.output) || toolCall.status === "running";
  }
  if (family === "delegated") {
    return Boolean(
      toolCall.delegated?.agents?.length ||
        toolCall.delegated?.message ||
        toolCall.output,
    );
  }
  return Boolean(toolCall.output);
}

function isPolicyRedirectOutput(toolCall: ToolCall): boolean {
  return (
    toolCall.status === "error" &&
    Boolean(
      toolCall.output?.includes("not allowed here") &&
        toolCall.output?.includes("use the dedicated"),
    )
  );
}

function extractExitLabel(output?: string): string | undefined {
  if (!output) return undefined;
  const exit = /exit(?:\s*code)?\s*[:=]?\s*(-?\d+)/i.exec(output);
  if (exit?.[1]) {
    return Number(exit[1]) === 0 ? "exit 0" : `exit ${exit[1]}`;
  }
  const first = output
    .split("\n")
    .map((line) => line.trim())
    .find(Boolean);
  if (!first) return undefined;
  if (first.length <= 24) return first;
  return undefined;
}

function firstString(...values: unknown[]): string | undefined {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return undefined;
}
