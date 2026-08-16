import { StyleSheet, Text, View } from "react-native";
import Markdown from "@ronradtke/react-native-markdown-display";
import { useThemeContext } from "../../hooks/useTheme";
import type { Report } from "@mitsuro/api";

interface ReportDetailContentProps {
  report: Report;
}

function formatProjectLabel(path?: string | null): string {
  if (!path) {
    return "No project selected";
  }

  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) {
    return path;
  }

  return parts.slice(-2).join("/");
}

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return "Pending";
  }

  return new Date(value).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function ReportDetailContent({ report }: ReportDetailContentProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const markdownStyles = {
    body: { color: t.foreground, fontSize: 15, lineHeight: 22 },
    heading1: {
      color: t.foreground,
      fontSize: 22,
      fontWeight: "700" as const,
      marginBottom: 8,
      marginTop: 16,
    },
    heading2: {
      color: t.foreground,
      fontSize: 19,
      fontWeight: "600" as const,
      marginBottom: 6,
      marginTop: 14,
    },
    heading3: {
      color: t.foreground,
      fontSize: 17,
      fontWeight: "600" as const,
      marginBottom: 4,
      marginTop: 12,
    },
    paragraph: { color: t.foreground, marginBottom: 10 },
    link: { color: t.userMessage },
    blockquote: {
      backgroundColor: t.glass.background,
      borderLeftColor: t.mutedForeground,
      borderLeftWidth: 3,
      paddingLeft: 12,
      paddingVertical: 4,
    },
    code_inline: {
      backgroundColor: t.glass.backgroundElevated,
      color: t.thinking,
      fontSize: 13,
      paddingHorizontal: 4,
      borderRadius: 3,
    },
    fence: {
      backgroundColor: t.codeSurface,
      padding: 12,
      borderRadius: 8,
      marginVertical: 8,
    },
    code_block: { color: t.foreground, fontSize: 13 },
    list_item: { color: t.foreground, marginBottom: 4 },
    bullet_list_icon: { color: t.mutedForeground },
    ordered_list_icon: { color: t.mutedForeground },
    hr: { backgroundColor: t.border, marginVertical: 16 },
    strong: { fontWeight: "700" as const },
    em: { fontStyle: "italic" as const },
  };

  return (
    <View>
      <View style={styles.metaRow}>
        <Text style={[styles.metaText, { color: t.mutedForeground }]}>
          {formatProjectLabel(report.project_dir)}
        </Text>
        <Text style={[styles.metaText, { color: t.mutedForeground }]}>
          {formatTimestamp(report.created_at)}
        </Text>
      </View>

      {report.tags.length > 0 ? (
        <View style={styles.tagRow}>
          {report.tags.map((tag) => (
            <View
              key={tag}
              style={[
                styles.tagPill,
                {
                  backgroundColor: `${t.userMessage}18`,
                  borderColor: `${t.userMessage}30`,
                },
              ]}
            >
              <Text style={[styles.tagText, { color: t.userMessage }]}>{tag}</Text>
            </View>
          ))}
        </View>
      ) : null}

      <Markdown style={markdownStyles}>{report.content}</Markdown>

      {report.sources.length > 0 ? (
        <View
          style={[
            styles.sourcesSection,
            { borderTopColor: theme.colors.border },
          ]}
        >
          <Text style={[styles.sourcesTitle, { color: t.mutedForeground }]}>
            Sources
          </Text>
          {report.sources.map((source) => (
            <Text key={source} style={[styles.sourceItem, { color: t.foreground }]}>
              {source}
            </Text>
          ))}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  metaRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
    marginBottom: 16,
  },
  metaText: {
    flex: 1,
    fontSize: 12,
    lineHeight: 16,
  },
  tagRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 6,
    marginBottom: 16,
  },
  tagPill: {
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
  },
  tagText: {
    fontSize: 11,
    fontWeight: "600",
  },
  sourcesSection: {
    marginTop: 24,
    paddingTop: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  sourcesTitle: {
    fontSize: 14,
    fontWeight: "600",
    marginBottom: 8,
  },
  sourceItem: {
    fontSize: 13,
    lineHeight: 18,
    marginBottom: 4,
  },
});
