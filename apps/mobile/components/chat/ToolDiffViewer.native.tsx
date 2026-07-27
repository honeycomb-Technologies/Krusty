import { useEffect, useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { highlightDiffRows, type NativeDiffToken } from "./nativeDiffHighlighter";
import {
  buildToolDiffRows,
  type ToolDiffPresentation,
  type ToolDiffRange,
  type ToolDiffRow,
} from "./toolDiffModel";

interface ToolDiffViewerProps {
  presentation: ToolDiffPresentation;
  rows?: ToolDiffRow[];
  showHeader?: boolean;
  maxLines?: number;
}

const MAX_NATIVE_LINES = 120;

export function ToolDiffViewer({
  presentation,
  rows: providedRows,
  showHeader = true,
  maxLines = MAX_NATIVE_LINES,
}: ToolDiffViewerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const rows = useMemo(
    () => providedRows ?? buildToolDiffRows(presentation),
    [presentation, providedRows],
  );
  const visibleRows = useMemo(() => rows.slice(0, maxLines), [maxLines, rows]);
  const [tokens, setTokens] = useState<NativeDiffToken[][] | null>(null);

  useEffect(() => {
    let current = true;
    setTokens(null);
    void highlightDiffRows(visibleRows, presentation.filePath, theme.scheme).then(
      (highlighted) => {
        if (current) setTokens(highlighted);
      },
    );
    return () => {
      current = false;
    };
  }, [presentation.filePath, theme.scheme, visibleRows]);

  return (
    <View style={[styles.frame, { borderColor: t.border, backgroundColor: t.card }]}>
      {showHeader ? (
        <View style={[styles.header, { borderBottomColor: t.border }]}>
          <Text style={[styles.path, { color: t.mutedForeground }]} numberOfLines={1}>
            {presentation.filePath || "Changes"}
          </Text>
          <Text style={[styles.stat, { color: t.success }]}>+{presentation.additions}</Text>
          <Text style={[styles.stat, { color: t.error }]}>-{presentation.deletions}</Text>
        </View>
      ) : null}
      <ScrollView horizontal nestedScrollEnabled contentContainerStyle={styles.content}>
        <View>
          {visibleRows.map((row, index) => (
            <DiffRow
              key={`${index}-${row.kind}-${row.content}`}
              row={row}
              tokens={tokens?.[index]}
              colors={t}
            />
          ))}
          {rows.length > maxLines ? (
            <Text style={[styles.truncated, { color: t.mutedForeground }]}>
              {rows.length - maxLines} more lines
            </Text>
          ) : null}
        </View>
      </ScrollView>
    </View>
  );
}

function DiffRow({
  row,
  tokens,
  colors,
}: {
  row: ToolDiffRow;
  tokens?: NativeDiffToken[];
  colors: ReturnType<typeof useThemeContext>["theme"]["colors"];
}) {
  const backgroundColor =
    row.kind === "addition"
      ? `${colors.success}14`
      : row.kind === "deletion"
        ? `${colors.error}14`
        : row.kind === "metadata"
          ? colors.muted
          : "transparent";
  const fallbackColor =
    row.kind === "metadata" ? colors.info : colors.foreground;
  const markerColor =
    row.kind === "addition"
      ? colors.success
      : row.kind === "deletion"
        ? colors.error
        : colors.mutedForeground;

  return (
    <View style={[styles.line, { backgroundColor }]}>
      <Text style={[styles.gutter, { color: colors.mutedForeground }]}>
        {row.oldLine ?? ""}
      </Text>
      <Text style={[styles.gutter, { color: colors.mutedForeground }]}>
        {row.newLine ?? ""}
      </Text>
      <Text style={[styles.marker, { color: markerColor }]}>{row.prefix || " "}</Text>
      <Text style={[styles.code, { color: fallbackColor }]} selectable>
        {row.kind === "metadata" || !tokens
          ? row.content || " "
          : renderTokens(tokens, row.changedRanges, colors.info)}
      </Text>
    </View>
  );
}

function renderTokens(
  tokens: NativeDiffToken[],
  ranges: ToolDiffRange[] | undefined,
  emphasisColor: string,
) {
  let offset = 0;
  return tokens.flatMap((token, tokenIndex) => {
    const fragments = splitToken(token, offset, ranges ?? []);
    offset += token.content.length;
    return fragments.map((fragment, fragmentIndex) => (
      <Text
        key={`${tokenIndex}-${fragmentIndex}`}
        style={{
          color: token.color,
          backgroundColor: fragment.changed ? `${emphasisColor}35` : "transparent",
        }}
      >
        {fragment.content}
      </Text>
    ));
  });
}

function splitToken(
  token: NativeDiffToken,
  tokenStart: number,
  ranges: ToolDiffRange[],
): { content: string; changed: boolean }[] {
  const boundaries = new Set([0, token.content.length]);
  for (const range of ranges) {
    const start = Math.max(0, range.start - tokenStart);
    const end = Math.min(token.content.length, range.end - tokenStart);
    if (start < end) {
      boundaries.add(start);
      boundaries.add(end);
    }
  }
  const points = [...boundaries].sort((a, b) => a - b);
  return points.slice(0, -1).map((start, index) => {
    const end = points[index + 1]!;
    const absoluteStart = tokenStart + start;
    return {
      content: token.content.slice(start, end),
      changed: ranges.some(
        (range) => absoluteStart < range.end && tokenStart + end > range.start,
      ),
    };
  });
}

const styles = StyleSheet.create({
  frame: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 9, overflow: "hidden" },
  header: {
    minHeight: 34,
    paddingHorizontal: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  path: { flex: 1, fontFamily: "Courier", fontSize: 11 },
  stat: { fontFamily: "Courier", fontSize: 11 },
  content: { minWidth: "100%", paddingVertical: 4 },
  line: { minHeight: 18, paddingRight: 8, flexDirection: "row", alignItems: "center" },
  gutter: {
    width: 34,
    paddingRight: 6,
    textAlign: "right",
    fontFamily: "Courier",
    fontSize: 10,
  },
  marker: { width: 16, textAlign: "center", fontFamily: "Courier", fontSize: 11 },
  code: { fontFamily: "Courier", fontSize: 11, lineHeight: 17 },
  truncated: { fontFamily: "Courier", fontSize: 11, padding: 8 },
});
