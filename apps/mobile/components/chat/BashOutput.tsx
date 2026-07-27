import { View, Text, ScrollView, StyleSheet } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";

interface BashOutputProps {
  command?: string;
  output: string;
  /** Compact live/error tail instead of full log panel. */
  mode?: "tail" | "log";
  maxTailLines?: number;
}

export function BashOutput({
  command,
  output,
  mode = "log",
  maxTailLines = 2,
}: BashOutputProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const body = mode === "tail" ? tailLines(output, maxTailLines) : output;

  if (mode === "tail") {
    if (!body && !command) return null;
    return (
      <View style={styles.tailWrap}>
        {command ? (
          <Text
            style={[styles.tailCommand, { color: t.mutedForeground }]}
            numberOfLines={1}
            selectable
          >
            $ {command}
          </Text>
        ) : null}
        {body ? (
          <Text
            style={[styles.tailOutput, { color: t.mutedForeground }]}
            numberOfLines={maxTailLines}
            selectable
          >
            {body}
          </Text>
        ) : null}
      </View>
    );
  }

  return (
    <View style={[styles.log, { borderColor: t.border, backgroundColor: t.card }]}>
      <ScrollView
        style={styles.body}
        horizontal={false}
        nestedScrollEnabled
        showsVerticalScrollIndicator={false}
      >
        {command ? (
          <Text style={styles.commandLine} selectable>
            <Text style={[styles.prompt, { color: t.success }]}>$ </Text>
            <Text style={[styles.command, { color: t.foreground }]}>{command}</Text>
          </Text>
        ) : null}
        {output ? (
          <Text style={[styles.output, { color: t.mutedForeground }]} selectable>
            {output}
          </Text>
        ) : (
          <Text style={[styles.output, { color: t.mutedForeground }]}>No output</Text>
        )}
      </ScrollView>
    </View>
  );
}

function tailLines(output: string, maxLines: number): string {
  if (!output) return "";
  const lines = output.replace(/\s+$/u, "").split("\n");
  if (lines.length <= maxLines) return lines.join("\n");
  return lines.slice(-maxLines).join("\n");
}

const styles = StyleSheet.create({
  tailWrap: {
    marginTop: 4,
    gap: 2,
    paddingLeft: 22,
  },
  tailCommand: {
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 15,
  },
  tailOutput: {
    fontFamily: "Courier",
    fontSize: 11,
    lineHeight: 15,
    opacity: 0.9,
  },
  log: {
    borderRadius: 8,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: 6,
  },
  body: {
    paddingHorizontal: 10,
    paddingVertical: 8,
    maxHeight: 220,
  },
  commandLine: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
    marginBottom: 4,
  },
  prompt: {
    fontFamily: "Courier",
  },
  command: {
    fontFamily: "Courier",
  },
  output: {
    fontFamily: "Courier",
    fontSize: 12,
    lineHeight: 17,
  },
});
