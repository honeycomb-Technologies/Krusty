import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { formatProjectLabel } from "./utils";
import type { MakoKnowledgeScope } from "./types";

interface MakoKnowledgeScopeToggleProps {
  activeScope: MakoKnowledgeScope;
  workspaceDirectory?: string | null;
  allLabel: string;
  allHint: string;
  onSelect: (scope: MakoKnowledgeScope) => void;
}

export function MakoKnowledgeScopeToggle({
  activeScope,
  workspaceDirectory,
  allLabel,
  allHint,
  onSelect,
}: MakoKnowledgeScopeToggleProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={styles.scopeRow}>
      {workspaceDirectory ? (
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            onSelect("workspace");
          }}
          style={[
            styles.scopePill,
            {
              backgroundColor:
                activeScope === "workspace" ? `${t.userMessage}18` : "transparent",
              borderColor:
                activeScope === "workspace" ? `${t.userMessage}44` : t.border,
            },
          ]}
        >
          <Text
            style={[
              styles.scopeLabel,
              {
                color:
                  activeScope === "workspace" ? t.userMessage : t.mutedForeground,
              },
            ]}
          >
            Current workspace
          </Text>
          <Text style={[styles.scopeHint, { color: t.mutedForeground }]}>
            {formatProjectLabel(workspaceDirectory)}
          </Text>
        </Pressable>
      ) : null}

      <Pressable
        onPress={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onSelect("all");
        }}
        style={[
          styles.scopePill,
          {
            backgroundColor:
              activeScope === "all" ? `${t.userMessage}18` : "transparent",
            borderColor: activeScope === "all" ? `${t.userMessage}44` : t.border,
          },
        ]}
      >
        <Text
          style={[
            styles.scopeLabel,
            {
              color: activeScope === "all" ? t.userMessage : t.mutedForeground,
            },
          ]}
        >
          {allLabel}
        </Text>
        <Text style={[styles.scopeHint, { color: t.mutedForeground }]}>
          {allHint}
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  scopeRow: {
    flexDirection: "row",
    gap: 12,
  },
  scopePill: {
    flex: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 18,
    paddingHorizontal: 14,
    paddingVertical: 12,
    gap: 2,
  },
  scopeLabel: {
    fontSize: 13,
    fontWeight: "700",
  },
  scopeHint: {
    fontSize: 11,
    lineHeight: 15,
  },
});
