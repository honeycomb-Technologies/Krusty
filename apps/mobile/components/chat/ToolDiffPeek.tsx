import { StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import type { ToolDiffRow } from "./toolDiffModel";

interface ToolDiffPeekProps {
  rows: ToolDiffRow[];
}

/** Lightweight cross-platform changed-line preview. Full diffs stay in ToolDiffViewer. */
export function ToolDiffPeek({ rows }: ToolDiffPeekProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.frame, { borderColor: t.border, backgroundColor: t.card }]}>
      {rows.map((row, index) => {
        const backgroundColor =
          row.kind === "addition"
            ? `${t.success}14`
            : row.kind === "deletion"
              ? `${t.error}14`
              : "transparent";
        const markerColor =
          row.kind === "addition"
            ? t.success
            : row.kind === "deletion"
              ? t.error
              : t.mutedForeground;
        const textColor =
          row.kind === "metadata" ? t.info : t.foreground;

        return (
          <View
            key={`${index}-${row.kind}-${row.content}`}
            style={[styles.line, { backgroundColor }]}
          >
            <Text style={[styles.marker, { color: markerColor }]}>
              {row.prefix || " "}
            </Text>
            <Text style={[styles.code, { color: textColor }]} numberOfLines={1} selectable>
              {row.content || " "}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    overflow: "hidden",
    paddingVertical: 4,
  },
  line: {
    minHeight: 18,
    paddingHorizontal: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  marker: {
    width: 12,
    textAlign: "center",
    fontFamily: "Courier",
    fontSize: 11,
  },
  code: {
    flex: 1,
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 16,
  },
});
