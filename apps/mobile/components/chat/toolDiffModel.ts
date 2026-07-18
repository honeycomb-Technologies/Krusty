import type { ToolCall } from "@krusty/api";

export type ToolDiffPresentation =
  | {
      kind: "patch";
      patch: string;
      filePath?: string;
      summary?: string;
      additions: number;
      deletions: number;
    }
  | {
      kind: "files";
      oldFile: { name: string; contents: string };
      newFile: { name: string; contents: string };
      filePath?: string;
      summary?: string;
      additions: number;
      deletions: number;
    };

const DIFF_TOOL_NAMES = new Set([
  "edit",
  "write",
  "multiedit",
  "multi_edit",
  "apply_patch",
  "patch",
]);

export function isDiffTool(toolName: string): boolean {
  return DIFF_TOOL_NAMES.has(toolName.toLowerCase());
}

export function buildToolDiffPresentation(
  toolCall: ToolCall,
): ToolDiffPresentation | null {
  if (!isDiffTool(toolCall.name)) return null;

  const args = toolCall.arguments ?? {};
  const filePath = firstString(args.file_path, args.path);
  const envelope = parseToolEnvelope(toolCall.output);
  const emittedPatch = envelope?.diff ?? rawUnifiedPatch(toolCall.output);

  if (emittedPatch) {
    const stats = countPatchChanges(emittedPatch);
    return {
      kind: "patch",
      patch: emittedPatch,
      filePath,
      summary: envelope?.summary,
      ...stats,
    };
  }

  const normalizedName = toolCall.name.toLowerCase();
  if (normalizedName === "apply_patch" || normalizedName === "patch") {
    const patch = convertApplyPatchToUnified(firstString(args.patch));
    if (!patch) return null;
    return {
      kind: "patch",
      patch,
      summary: envelope?.summary,
      ...countPatchChanges(patch),
    };
  }

  if (normalizedName === "edit") {
    const oldContents = firstString(args.old_string);
    const newContents = firstString(args.new_string);
    if (oldContents === undefined || newContents === undefined) return null;
    const name = filePath || "edited-file";
    return {
      kind: "files",
      oldFile: { name, contents: oldContents },
      newFile: { name, contents: newContents },
      filePath,
      summary: envelope?.summary,
      additions: lineCount(newContents),
      deletions: lineCount(oldContents),
    };
  }

  if (normalizedName === "write") {
    const contents = firstString(args.content);
    if (contents === undefined) return null;
    const name = filePath || "new-file";
    return {
      kind: "files",
      oldFile: { name, contents: "" },
      newFile: { name, contents },
      filePath,
      summary: envelope?.summary,
      additions: lineCount(contents),
      deletions: 0,
    };
  }

  return null;
}

function firstString(...values: unknown[]): string | undefined {
  return values.find((value): value is string => typeof value === "string");
}

function lineCount(value: string): number {
  if (!value) return 0;
  return value.endsWith("\n")
    ? Math.max(0, value.split("\n").length - 1)
    : value.split("\n").length;
}

function rawUnifiedPatch(output?: string): string | undefined {
  if (!output) return undefined;
  const trimmed = output.trimStart();
  return trimmed.startsWith("--- ") && trimmed.includes("\n+++ ")
    ? output
    : undefined;
}

function parseToolEnvelope(
  output?: string,
): { diff?: string; summary?: string } | null {
  if (!output) return null;
  try {
    const parsed = JSON.parse(output) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    const data =
      parsed.data && typeof parsed.data === "object" && !Array.isArray(parsed.data)
        ? (parsed.data as Record<string, unknown>)
        : undefined;
    return {
      diff: firstString(parsed.diff),
      summary: firstString(data?.message, parsed.summary),
    };
  } catch {
    return null;
  }
}

function countPatchChanges(patch: string): {
  additions: number;
  deletions: number;
} {
  let additions = 0;
  let deletions = 0;
  for (const line of patch.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }
  return { additions, deletions };
}

/** Convert Krusty's apply_patch envelope into a display-only unified patch. */
function convertApplyPatchToUnified(patch?: string): string | undefined {
  if (!patch?.includes("*** Begin Patch")) return undefined;

  const lines = patch.split("\n");
  const output: string[] = [];
  let index = 0;

  while (index < lines.length) {
    const header = lines[index]?.trim() ?? "";
    const updatePath = header.startsWith("*** Update File: ")
      ? header.slice("*** Update File: ".length).trim()
      : undefined;
    const addPath = header.startsWith("*** Add File: ")
      ? header.slice("*** Add File: ".length).trim()
      : undefined;

    if (!updatePath && !addPath) {
      index += 1;
      continue;
    }

    index += 1;
    const body: string[] = [];
    while (index < lines.length && !lines[index]?.trim().startsWith("*** ")) {
      const line = lines[index] ?? "";
      if (!line.startsWith("@@")) {
        body.push(line.startsWith("+") || line.startsWith("-") || line.startsWith(" ") ? line : ` ${line}`);
      }
      index += 1;
    }

    const additions = body.filter((line) => line.startsWith("+")).length;
    const deletions = body.filter((line) => line.startsWith("-")).length;
    const context = body.length - additions - deletions;
    const path = updatePath ?? addPath ?? "file";
    output.push(
      addPath ? "--- /dev/null" : `--- a/${path}`,
      `+++ b/${path}`,
      `@@ -${addPath ? 0 : 1},${addPath ? 0 : deletions + context} +1,${additions + context} @@`,
      ...body,
    );
  }

  return output.length > 0 ? `${output.join("\n")}\n` : undefined;
}

