import { type ComponentProps, memo } from "react";
import {
  FlatList,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Animated from "react-native-reanimated";
import * as Haptics from "../../platform/haptics";
import type { ModelInfo } from "@mitsuro/api";
import { modelKeysEqual } from "@mitsuro/state";
import { AdaptiveMaterial } from "../ui/AdaptiveMaterial";

const RADIUS = 18;
/**
 * The accordion responder spans the full composer width so its provider dock
 * can extend left of the FAB column. Keep the model list above that responder:
 * otherwise iOS sends vertical pans to the accordion's GestureDetector instead
 * of the FlatList even though the transparent rows use `box-none`.
 */
const MODEL_POPOVER_Z_INDEX = 45;

export interface ChatBarModelPopoverProps {
  interactive: boolean;
  modelPopoverHeight: number;
  modelPopoverClipStyle: ComponentProps<typeof Animated.View>["style"];
  modelPopoverShellStyle: ComponentProps<typeof Animated.View>["style"];
  modelPopoverContentStyle: ComponentProps<typeof Animated.View>["style"];
  modelPopoverCoverStyle: ComponentProps<typeof Animated.View>["style"];
  materialActive: boolean;
  nativeGlassActive: boolean;
  borderColor: string;
  foreground: string;
  mutedForeground: string;
  thinking: string;
  backgroundPressed: string;
  surfaceColor: string;
  filteredModels: ModelInfo[];
  selectedModel: ModelInfo | null;
  onSelectModel: (model: ModelInfo) => void;
}

function modelRowKey(model: ModelInfo, index: number): string {
  const key = model.key;
  return key
    ? [key.provider, key.model_id, key.auth_scope ?? "", key.api_format ?? ""]
      .join("\u0000")
    : [model.provider ?? "", model.id, String(index)].join("\u0000");
}

function isSelectedModel(item: ModelInfo, selected: ModelInfo | null): boolean {
  if (!selected) return false;
  if (item.key || selected.key) {
    return modelKeysEqual(item.key ?? null, selected.key ?? null);
  }
  return item.id === selected.id && item.provider === selected.provider;
}

function ChatBarModelPopoverComponent({
  interactive,
  modelPopoverHeight,
  modelPopoverClipStyle,
  modelPopoverShellStyle,
  modelPopoverContentStyle,
  modelPopoverCoverStyle,
  materialActive,
  nativeGlassActive,
  borderColor,
  foreground,
  mutedForeground,
  thinking,
  backgroundPressed,
  surfaceColor,
  filteredModels,
  selectedModel,
  onSelectModel,
}: ChatBarModelPopoverProps) {
  if (modelPopoverHeight <= 0) return null;

  return (
    <Animated.View
      pointerEvents={interactive ? "box-none" : "none"}
      style={[
        styles.modelClip,
        modelPopoverClipStyle,
        { borderColor: nativeGlassActive ? "transparent" : borderColor },
      ]}
    >
      {nativeGlassActive ? null : (
        <AdaptiveMaterial
          active={materialActive}
          borderRadius={RADIUS}
          tone="elevated"
          fallbackColor={surfaceColor}
          liquidGlassOnly
          respectMotionGate
        />
      )}
      {nativeGlassActive ? null : Platform.OS === "ios" ? (
        <Animated.View
          pointerEvents="none"
          style={[
            StyleSheet.absoluteFill,
            { borderRadius: RADIUS, backgroundColor: surfaceColor },
            modelPopoverCoverStyle,
          ]}
        />
      ) : (
        <View
          pointerEvents="none"
          style={[
            StyleSheet.absoluteFill,
            { borderRadius: RADIUS, backgroundColor: surfaceColor },
          ]}
        />
      )}
      <Animated.View
        style={[
          styles.modelPopover,
          modelPopoverShellStyle,
        ]}
      >
        <Animated.View
          style={[{ height: modelPopoverHeight }, modelPopoverContentStyle]}
        >
          <FlatList
            data={filteredModels}
            keyExtractor={modelRowKey}
            style={styles.modelList}
            contentContainerStyle={styles.modelListContent}
            extraData={selectedModel}
            nestedScrollEnabled
            removeClippedSubviews={false}
            keyboardShouldPersistTaps="handled"
            keyboardDismissMode="none"
            showsVerticalScrollIndicator={false}
            renderItem={({ item }: { item: ModelInfo }) => {
              const selected = isSelectedModel(item, selectedModel);
              return (
                <Pressable
                  accessibilityRole="button"
                  accessibilityState={{ selected }}
                  onPress={() => {
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                    onSelectModel(item);
                  }}
                  style={({ pressed }) => [
                    styles.modelItem,
                    pressed && { backgroundColor: backgroundPressed },
                  ]}
                >
                  <View style={styles.modelRow}>
                    <View style={styles.modelInfo}>
                      <Text
                        style={[styles.modelName, { color: foreground }]}
                        numberOfLines={1}
                      >
                        {item.display_name}
                      </Text>
                      <Text
                        style={[styles.modelMeta, { color: mutedForeground }]}
                      >
                        {item.provider}
                      </Text>
                    </View>
                    {selected && (
                      <Text style={[styles.modelCheck, { color: thinking }]}>
                        ✓
                      </Text>
                    )}
                  </View>
                </Pressable>
              );
            }}
          />
        </Animated.View>
      </Animated.View>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  // The shared progress morphs this clip from the model FAB to the final list.
  // Its fixed-size child stays aligned to the final endpoint, so FlatList never
  // changes viewport geometry while the surface performs the genie motion.
  modelClip: {
    position: "absolute",
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    zIndex: MODEL_POPOVER_Z_INDEX,
  },
  // Fixed-size list content is positioned inside the morphing clip.
  modelPopover: {
    position: "absolute",
    borderRadius: RADIUS,
    overflow: "hidden",
  },
  modelList: {
    flex: 1,
    paddingHorizontal: 8,
  },
  modelListContent: {
    paddingTop: 8,
    paddingBottom: 10,
  },
  modelItem: {
    paddingHorizontal: 14,
    paddingVertical: 12,
    borderRadius: 12,
    marginBottom: 4,
  },
  modelRow: { flexDirection: "row", alignItems: "center" },
  modelInfo: { flex: 1 },
  modelName: { fontSize: 15, fontWeight: "500" },
  modelMeta: { fontSize: 12, marginTop: 2 },
  modelCheck: { fontSize: 18, fontWeight: "700", marginLeft: 8 },
});

export const ChatBarModelPopover = memo(ChatBarModelPopoverComponent);
