import { Image, Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { X } from "lucide-react-native";
import { AdaptiveMaterial } from "../ui/AdaptiveMaterial";
import { useThemeContext } from "../../hooks/useTheme";

export interface ImagePreviewCandidate {
  uri?: string | null;
  base64?: string | null;
  mimeType?: string | null;
}

interface ImagePreviewModalProps {
  visible: boolean;
  uri?: string | null;
  title?: string | null;
  onClose: () => void;
}

export function imagePreviewUri(
  image: ImagePreviewCandidate | null | undefined,
): string | null {
  const uri = image?.uri?.trim();
  if (uri) return uri;

  const base64 = image?.base64?.trim();
  if (!base64) return null;
  if (base64.startsWith("data:")) return base64;

  const mimeType = image?.mimeType?.trim() || "image/png";
  return `data:${mimeType};base64,${base64}`;
}

export function ImagePreviewModal({
  visible,
  uri,
  title,
  onClose,
}: ImagePreviewModalProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const previewVisible = visible && Boolean(uri);

  return (
    <Modal
      visible={previewVisible}
      transparent
      animationType="fade"
      onRequestClose={onClose}
    >
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View style={[styles.card, { borderColor: t.border }]}>
          <AdaptiveMaterial tone="strong" borderRadius={24} />

          <View style={[styles.header, { borderBottomColor: t.border }]}>
            <Text
              style={[styles.title, { color: t.foreground }]}
              numberOfLines={1}
            >
              {title || "Image preview"}
            </Text>
            <Pressable
              onPress={onClose}
              hitSlop={10}
              style={({ pressed }) => [
                styles.closeButton,
                {
                  backgroundColor: pressed
                    ? t.glass.backgroundPressed
                    : t.glass.backgroundElevated,
                },
              ]}
            >
              <X size={18} color={t.foreground} strokeWidth={2.2} />
            </Pressable>
          </View>

          <View style={styles.imageWrap}>
            {uri ? (
              <Image
                source={{ uri }}
                style={styles.image}
                resizeMode="contain"
                accessibilityLabel={title || "Image preview"}
              />
            ) : null}
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(0,0,0,0.72)",
    padding: 18,
  },
  card: {
    width: "94%",
    height: "78%",
    maxWidth: 960,
    maxHeight: 760,
    minHeight: 280,
    borderRadius: 24,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
  },
  header: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  title: {
    flex: 1,
    fontSize: 15,
    fontWeight: "700",
  },
  closeButton: {
    width: 34,
    height: 34,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
  },
  imageWrap: {
    flex: 1,
    padding: 14,
  },
  image: {
    width: "100%",
    height: "100%",
  },
});
