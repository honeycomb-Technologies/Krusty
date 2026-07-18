import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import type { ToolDiffPresentation } from "./toolDiffModel";

interface ToolDiffViewerProps {
  presentation: ToolDiffPresentation;
}

const MAX_NATIVE_LINES = 180;

export function ToolDiffViewer({ presentation }: ToolDiffViewerProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const lines = diffLines(presentation);
  const visibleLines = lines.slice(0, MAX_NATIVE_LINES);

  return (
    <View style={[styles.frame, { borderColor: t.border, backgroundColor: t.card }]}>
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <Text style={[styles.path, { color: t.mutedForeground }]} numberOfLines={1}>
          {presentation.filePath || "Changes"}
        </Text>
        <Text style={styles.added}>+{presentation.additions}</Text>
        <Text style={styles.removed}>-{presentation.deletions}</Text>
      </View>
      <ScrollView horizontal nestedScrollEnabled contentContainerStyle={styles.content}>
        <View>
          {visibleLines.map((line, index) => {
            const kind = lineKind(line);
            const color =
              kind === "addition"
                ? t.success
                : kind === "deletion"
                  ? t.error
                  : kind === "metadata"
                    ? t.info
                    : t.foreground;
            const backgroundColor =
              kind === "addition"
                ? `${t.success}14`
                : kind === "deletion"
                  ? `${t.error}14`
                  : "transparent";
            return (
              <View key={`${index}-${line}`} style={[styles.line, { backgroundColor }]}>
                <Text style={[styles.code, { color }]} selectable>
                  {line || " "}
                </Text>
              </View>
            );
          })}
          {lines.length > MAX_NATIVE_LINES ? (
            <Text style={[styles.truncated, { color: t.mutedForeground }]}>
              {lines.length - MAX_NATIVE_LINES} more lines
            </Text>
          ) : null}
        </View>
      </ScrollView>
    </View>
  );
}

function diffLines(presentation: ToolDiffPresentation): string[] {
  if (presentation.kind === "patch") return presentation.patch.trimEnd().split("\n");
  return [
    `--- ${presentation.oldFile.name}`,
    `+++ ${presentation.newFile.name}`,
    ...presentation.oldFile.contents.split("\n").map((line) => `-${line}`),
    ...presentation.newFile.contents.split("\n").map((line) => `+${line}`),
  ];
}

function lineKind(line: string): "addition" | "deletion" | "metadata" | "context" {
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("@@")) {
    return "metadata";
  }
  if (line.startsWith("+")) return "addition";
  if (line.startsWith("-")) return "deletion";
  return "context";
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
  added: { color: "#22c55e", fontFamily: "Courier", fontSize: 11 },
  removed: { color: "#ef4444", fontFamily: "Courier", fontSize: 11 },
  content: { minWidth: "100%", paddingVertical: 4 },
  line: { minHeight: 18, paddingHorizontal: 8, justifyContent: "center" },
  code: { fontFamily: "Courier", fontSize: 11, lineHeight: 17 },
  truncated: { fontFamily: "Courier", fontSize: 11, padding: 8 },
});

