import { View, Text, ScrollView, StyleSheet } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { DotEchoIndicator } from "./DotEchoIndicator";

const LIVE_TAIL_LINES = 3;
const EXPANDED_MAX_LINES = 40;

interface BashOutputProps {
  command?: string;
  output: string;
  /** True while the command is still running. */
  streaming?: boolean;
  /**
   * Hybrid density: short live tail (default while streaming).
   * Full scrollable body when expanded.
   */
  compact?: boolean;
}

export function BashOutput({
  command,
  output,
  streaming = false,
  compact = false,
}: BashOutputProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const lines = output.length > 0 ? output.replace(/\s+$/, "").split("\n") : [];
  const maxLines = compact ? LIVE_TAIL_LINES : EXPANDED_MAX_LINES;
  const visibleLines =
    lines.length > maxLines ? lines.slice(lines.length - maxLines) : lines;
  const truncated = lines.length > maxLines;

  return (
    <View
      style={[
        styles.shell,
        compact ? styles.shellCompact : styles.shellExpanded,
        { borderColor: t.border },
      ]}
    >
      <View style={styles.header}>
        <Text style={[styles.caret, { color: t.primary }]} numberOfLines={1}>
          {compact || streaming ? "▼" : "▶"}
        </Text>
        <Text style={[styles.prompt, { color: t.primary }]}>$</Text>
        <Text
          style={[styles.command, { color: t.foreground }]}
          numberOfLines={1}
        >
          {command?.trim() || "bash"}
        </Text>
        {streaming ? (
          <View style={styles.activity}>
            <DotEchoIndicator color={t.thinking} />
          </View>
        ) : null}
      </View>

      {visibleLines.length > 0 ? (
        <ScrollView
          style={compact ? styles.bodyCompact : styles.bodyExpanded}
          horizontal={false}
          showsVerticalScrollIndicator={!compact}
        >
          {truncated && !compact ? (
            <Text style={[styles.meta, { color: t.mutedForeground }]}>
              …earlier output hidden
            </Text>
          ) : null}
          <Text
            style={[styles.output, { color: t.foreground }]}
            selectable
          >
            {visibleLines.join("\n")}
          </Text>
        </ScrollView>
      ) : streaming ? (
        <Text style={[styles.meta, { color: t.mutedForeground }]}>
          running…
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  shell: {
    backgroundColor: "rgba(0,0,0,0.28)",
    borderRadius: 8,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    marginVertical: 4,
  },
  shellCompact: {
    // hybrid: header + short live tail, not a full terminal pane
  },
  shellExpanded: {
    // keep a bit more room when the user expands for detail
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  caret: {
    fontSize: 11,
    fontWeight: "600",
  },
  prompt: {
    fontFamily: "Courier",
    fontSize: 13,
    fontWeight: "600",
  },
  command: {
    flex: 1,
    fontFamily: "Courier",
    fontSize: 13,
  },
  activity: {
    marginLeft: 4,
  },
  bodyCompact: {
    paddingHorizontal: 12,
    paddingBottom: 8,
    maxHeight: 72,
  },
  bodyExpanded: {
    paddingHorizontal: 12,
    paddingBottom: 10,
    maxHeight: 250,
  },
  meta: {
    fontFamily: "Courier",
    fontSize: 11,
    marginBottom: 2,
  },
  output: {
    fontFamily: "Courier",
    fontSize: 13,
    lineHeight: 18,
  },
});
