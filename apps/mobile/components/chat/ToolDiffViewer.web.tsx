import { useMemo } from "react";
import { FileDiff, MultiFileDiff } from "@pierre/diffs/react";
import {
  parsePatchFiles,
  registerCustomLanguage,
  registerCustomTheme,
} from "@pierre/diffs";
import rustLanguage from "@shikijs/langs/rust";
import { useThemeContext } from "../../hooks/useTheme";
import {
  MITSURO_DIFF_THEME_NAMES,
  mitsuroDarkDiffTheme,
  mitsuroLightDiffTheme,
} from "./mitsuroDiffThemes";
import type { ToolDiffPresentation } from "./toolDiffModel";

interface ToolDiffViewerProps {
  presentation: ToolDiffPresentation;
  /** Native peek support; web still renders full presentation when provided. */
  rows?: unknown;
  showHeader?: boolean;
  maxLines?: number;
}

const MITSURO_DIFF_CSS = `
  :host {
    --diffs-font-family: "SFMono-Regular", "SF Mono", ui-monospace, monospace;
    --diffs-header-font-family: Inter, ui-sans-serif, system-ui, sans-serif;
    --diffs-font-size: 12px;
    --diffs-line-height: 18px;
    --diffs-gap-inline: 8px;
    --diffs-gap-block: 5px;
  }
  [data-diffs-header="default"] {
    min-height: 34px;
    padding-inline: 10px;
    border-bottom: 1px solid var(--mitsuro-diff-border);
  }
  [data-separator="metadata"] {
    opacity: .82;
  }
`;

registerCustomTheme(MITSURO_DIFF_THEME_NAMES.dark, async () => mitsuroDarkDiffTheme);
registerCustomTheme(MITSURO_DIFF_THEME_NAMES.light, async () => mitsuroLightDiffTheme);
// Metro cannot safely bundle Pierre's runtime-generated absolute Rust grammar
// import when node_modules is shared by a worktree. Registering the grammar as
// a static module keeps the web diff viewer deterministic.
registerCustomLanguage("rust", async () => ({ default: rustLanguage }), ["rs"]);

export function ToolDiffViewer({ presentation }: ToolDiffViewerProps) {
  // rows/showHeader/maxLines are accepted for API parity with native peek mode.
  const { theme } = useThemeContext();
  const t = theme.colors;
  const options = useMemo(
    () => ({
      theme: MITSURO_DIFF_THEME_NAMES[theme.scheme],
      themeType: theme.scheme,
      diffStyle: "unified" as const,
      diffIndicators: "classic" as const,
      lineDiffType: "word" as const,
      hunkSeparators: "metadata" as const,
      overflow: "scroll" as const,
      unsafeCSS: MITSURO_DIFF_CSS,
    }),
    [theme.scheme],
  );
  const style = {
    "--diffs-dark-bg": t.card,
    "--diffs-light-bg": t.card,
    "--diffs-dark": t.foreground,
    "--diffs-light": t.foreground,
    "--diffs-addition-color": t.success,
    "--diffs-deletion-color": t.error,
    "--diffs-modified-color": t.info,
    "--diffs-bg-context-override": t.card,
    "--diffs-bg-separator-override": t.muted,
    "--mitsuro-diff-border": t.border,
  } as React.CSSProperties;
  const files = useMemo(
    () =>
      presentation.kind === "patch"
        ? parsePatchFiles(presentation.patch, undefined, false).flatMap(
            (item) => item.files,
          )
        : [],
    [presentation],
  );

  if (presentation.kind === "files") {
    return (
      <div style={wrapperStyle(t.border)} onClick={(event) => event.stopPropagation()}>
        <MultiFileDiff
          oldFile={presentation.oldFile}
          newFile={presentation.newFile}
          options={options}
          style={style}
        />
      </div>
    );
  }

  if (files.length === 0) return null;

  return (
    <div style={wrapperStyle(t.border)} onClick={(event) => event.stopPropagation()}>
      {files.map((fileDiff, index) => (
        <FileDiff
          key={`${fileDiff.name ?? presentation.filePath ?? "diff"}-${index}`}
          fileDiff={fileDiff}
          options={options}
          style={style}
        />
      ))}
    </div>
  );
}

function wrapperStyle(borderColor: string): React.CSSProperties {
  return {
    border: `1px solid ${borderColor}`,
    borderRadius: 9,
    overflow: "hidden",
    maxWidth: "100%",
  };
}
