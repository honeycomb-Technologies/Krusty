import { useState } from "react";
import { Code2, Copy, Eye } from "lucide-react-native";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import * as Clipboard from "../../platform/clipboard";
import * as Haptics from "../../platform/haptics";

interface HtmlPreviewProps {
  html: string;
}

const HTML_IFRAME_SANDBOX = "allow-scripts";

export function HtmlPreview({ html }: HtmlPreviewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [showSource, setShowSource] = useState(false);
  const [copied, setCopied] = useState(false);

  const copySource = () => {
    void Clipboard.setStringAsync(html);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <View
      style={[
        styles.container,
        { backgroundColor: t.card, borderColor: t.border },
      ]}
    >
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <Text style={[styles.label, { color: t.mutedForeground }]}>
          HTML
        </Text>
        <View style={styles.actions}>
          <PreviewButton
            active={!showSource}
            label="Preview"
            icon={<Eye size={13} color={showSource ? t.mutedForeground : t.foreground} />}
            onPress={() => setShowSource(false)}
            colors={t}
          />
          <PreviewButton
            active={showSource}
            label="Code"
            icon={<Code2 size={13} color={showSource ? t.foreground : t.mutedForeground} />}
            onPress={() => setShowSource(true)}
            colors={t}
          />
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={copied ? "HTML copied" : "Copy HTML"}
            onPress={copySource}
            style={({ pressed }) => [
              styles.action,
              pressed && styles.pressed,
            ]}
          >
            <Copy size={13} color={t.mutedForeground} />
            <Text style={[styles.actionLabel, { color: t.mutedForeground }]}>
              {copied ? "Copied" : "Copy"}
            </Text>
          </Pressable>
        </View>
      </View>

      {showSource ? (
        <ScrollView
          horizontal
          nestedScrollEnabled
          contentContainerStyle={styles.sourceContent}
        >
          <Text style={[styles.source, { color: t.foreground }]} selectable>
            {html}
          </Text>
        </ScrollView>
      ) : (
        <div style={{ width: "100%", height: 360, background: "#fff" }}>
          <iframe
            title="HTML preview"
            srcDoc={html}
            sandbox={HTML_IFRAME_SANDBOX}
            referrerPolicy="no-referrer"
            style={{
              display: "block",
              width: "100%",
              height: "100%",
              border: 0,
              background: "#fff",
            }}
          />
        </div>
      )}
    </View>
  );
}

function PreviewButton({
  active,
  label,
  icon,
  onPress,
  colors,
}: {
  active: boolean;
  label: string;
  icon: React.ReactNode;
  onPress: () => void;
  colors: ReturnType<typeof useThemeContext>["theme"]["colors"];
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: active }}
      onPress={onPress}
      style={({ pressed }) => [
        styles.action,
        active && { backgroundColor: colors.muted },
        pressed && styles.pressed,
      ]}
    >
      {icon}
      <Text
        style={[
          styles.actionLabel,
          { color: active ? colors.foreground : colors.mutedForeground },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  container: {
    width: "100%",
    marginVertical: 6,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    overflow: "hidden",
  },
  header: {
    minHeight: 38,
    paddingHorizontal: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  label: {
    fontSize: 11,
    fontFamily: "Courier",
    fontWeight: "600",
  },
  actions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  action: {
    minHeight: 28,
    paddingHorizontal: 7,
    borderRadius: 6,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
  },
  actionLabel: {
    fontSize: 11,
    fontWeight: "500",
  },
  pressed: {
    opacity: 0.62,
  },
  sourceContent: {
    minWidth: "100%",
    minHeight: 180,
    padding: 12,
  },
  source: {
    fontFamily: "Courier",
    fontSize: 13,
    lineHeight: 19,
  },
});
