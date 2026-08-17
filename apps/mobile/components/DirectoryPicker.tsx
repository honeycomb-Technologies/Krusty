import { colors } from '@mitsuro/ui';
import { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  Pressable,
  FlatList,
  Modal,
  StyleSheet,
  ActivityIndicator,
} from 'react-native';
import { Folder, ChevronLeft, X, Check } from 'lucide-react-native';
import * as Haptics from '../platform/haptics';
import { useThemeContext } from '../hooks/useTheme';
import { useConnection } from '../hooks/useConnection';

interface DirectoryEntry {
  name: string;
  path: string;
}

interface DirectoryPickerProps {
  visible: boolean;
  onSelect: (path: string) => void;
  onClose: () => void;
  initialPath?: string;
}

export function DirectoryPicker({ visible, onSelect, onClose, initialPath }: DirectoryPickerProps) {
  const { theme } = useThemeContext();
  const { client } = useConnection();
  const t = theme.colors;

  const [currentPath, setCurrentPath] = useState(initialPath ?? '');
  const [parentPath, setParentPath] = useState<string | null>(null);
  const [directories, setDirectories] = useState<DirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchDirectories = useCallback(async (path: string) => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      const result = await client.browseDirectories(path);
      setCurrentPath(result.current);
      setParentPath(result.parent);
      setDirectories(result.directories);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to browse directories');
      setDirectories([]);
    } finally {
      setLoading(false);
    }
  }, [client]);

  useEffect(() => {
    if (visible) {
      fetchDirectories(initialPath ?? '');
    }
  }, [visible, initialPath, fetchDirectories]);

  const navigateInto = useCallback((path: string) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    fetchDirectories(path);
  }, [fetchDirectories]);

  const navigateUp = useCallback(() => {
    if (!parentPath) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    fetchDirectories(parentPath);
  }, [parentPath, fetchDirectories]);

  const handleSelect = useCallback(() => {
    Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    onSelect(currentPath);
  }, [currentPath, onSelect]);

  const renderDirectoryItem = useCallback(({ item }: { item: DirectoryEntry }) => (
    <Pressable
      onPress={() => navigateInto(item.path)}
      style={({ pressed }) => [
        styles.dirRow,
        { backgroundColor: pressed ? t.surface : 'transparent' },
      ]}
    >
      <Folder size={20} color={t.thinking} strokeWidth={1.6} />
      <Text style={[styles.dirName, { color: t.foreground }]} numberOfLines={1}>
        {item.name}
      </Text>
    </Pressable>
  ), [navigateInto, t]);

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />

        <View style={styles.modalContainer}>
          <View
            style={[
              styles.card,
              { backgroundColor: t.background, borderColor: t.border },
            ]}
          >
            {/* Header */}
            <View style={[styles.header, { borderBottomColor: t.border }]}>
              <View style={styles.headerLeft}>
                <Text style={[styles.headerTitle, { color: t.foreground }]}>Select Directory</Text>
                <Text style={[styles.currentPath, { color: t.mutedForeground }]} numberOfLines={1}>
                  {currentPath}
                </Text>
              </View>
              <Pressable
                onPress={onClose}
                style={[styles.closeBtn, { backgroundColor: t.surface }]}
              >
                <X size={18} color={t.mutedForeground} strokeWidth={2} />
              </Pressable>
            </View>

            {/* Up button */}
            {parentPath && (
              <Pressable
                onPress={navigateUp}
                style={({ pressed }) => [
                  styles.upRow,
                  { borderBottomColor: t.border, backgroundColor: pressed ? t.surface : 'transparent' },
                ]}
              >
                <ChevronLeft size={20} color={t.primary} strokeWidth={2} />
                <Text style={[styles.upText, { color: t.primary }]}>Up</Text>
              </Pressable>
            )}

            {/* Directory list */}
            <View style={styles.listContainer}>
              {loading ? (
                <View style={styles.centered}>
                  <ActivityIndicator color={t.primary} size="small" />
                </View>
              ) : error ? (
                <View style={styles.centered}>
                  <Text style={[styles.errorText, { color: t.error }]}>{error}</Text>
                </View>
              ) : directories.length === 0 ? (
                <View style={styles.centered}>
                  <Text style={[styles.emptyText, { color: t.mutedForeground }]}>No subdirectories</Text>
                </View>
              ) : (
                <FlatList
                  data={directories}
                  keyExtractor={(item) => item.path}
                  renderItem={renderDirectoryItem}
                  showsVerticalScrollIndicator={false}
                  keyboardShouldPersistTaps="handled"
                />
              )}
            </View>

            {/* Select button */}
            <View style={[styles.footer, { borderTopColor: t.border }]}>
              <Pressable
                onPress={handleSelect}
                style={[styles.selectBtn, { backgroundColor: t.userMessage }]}
              >
                <Check size={18} color={t.onAccent} strokeWidth={2.5} />
                <Text style={styles.selectBtnText}>Select This Directory</Text>
              </Pressable>
            </View>
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.6)',
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  modalContainer: {
    width: '100%',
    maxHeight: '80%',
  },
  card: {
    borderRadius: 22,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 16,
    paddingTop: 16,
    paddingBottom: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerLeft: {
    flex: 1,
    marginRight: 12,
  },
  headerTitle: {
    fontSize: 18,
    fontWeight: '700',
  },
  currentPath: {
    fontSize: 12,
    marginTop: 2,
  },
  closeBtn: {
    width: 32,
    height: 32,
    borderRadius: 16,
    justifyContent: 'center',
    alignItems: 'center',
  },
  upRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  upText: {
    fontSize: 15,
    fontWeight: '600',
  },
  listContainer: {
    minHeight: 120,
    maxHeight: 360,
  },
  centered: {
    paddingVertical: 40,
    alignItems: 'center',
    justifyContent: 'center',
  },
  errorText: {
    fontSize: 14,
    textAlign: 'center',
    paddingHorizontal: 16,
  },
  emptyText: {
    fontSize: 14,
    textAlign: 'center',
  },
  dirRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 13,
  },
  dirName: {
    flex: 1,
    fontSize: 15,
    fontWeight: '500',
  },
  footer: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  selectBtn: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 8,
    paddingVertical: 12,
    borderRadius: 14,
  },
  selectBtnText: {
    color: colors.onAccent,
    fontSize: 16,
    fontWeight: '600',
  },
});
