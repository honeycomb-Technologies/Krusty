import { Pressable, StyleSheet, Text, View } from 'react-native';
import { Code2, MessageCircle, Settings } from 'lucide-react-native';
import { HiveSharkIcon } from '@mobile/components/ui/HiveSharkIcon';
import { useThemeContext } from '@mobile/hooks/useTheme';
import type { DesktopPlane } from './types';
import { DESKTOP } from './desktopTheme';

const PLANES: Array<{ id: DesktopPlane; label: string }> = [
  { id: 'chat', label: 'Chat' },
  { id: 'code', label: 'Code' },
  { id: 'hive', label: 'Hive' },
];

export function PlaneRail({
  plane,
  onPlaneChange,
  onOpenSettings,
  attentionCount = 0,
}: {
  plane: DesktopPlane;
  onPlaneChange: (plane: DesktopPlane) => void;
  onOpenSettings: () => void;
  attentionCount?: number;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.rail, { backgroundColor: t.background, borderRightColor: t.border }]}>
      <View style={styles.top}>
        {PLANES.map((item) => {
          const active = item.id === plane;
          const color = active ? t.foreground : t.mutedForeground;
          return (
            <Pressable
              key={item.id}
              accessibilityRole="tab"
              accessibilityState={{ selected: active }}
              accessibilityLabel={item.label}
              onPress={() => onPlaneChange(item.id)}
              style={[
                styles.item,
                active && {
                  backgroundColor: t.glass.backgroundElevated,
                  borderColor: `${t.userMessage}40`,
                },
              ]}
            >
              {item.id === 'chat' ? (
                <MessageCircle size={18} color={color} strokeWidth={active ? 2.2 : 1.8} />
              ) : null}
              {item.id === 'code' ? (
                <Code2 size={18} color={color} strokeWidth={active ? 2.2 : 1.8} />
              ) : null}
              {item.id === 'hive' ? (
                <HiveSharkIcon size={18} color={color} strokeWidth={active ? 2.2 : 1.8} />
              ) : null}
              {item.id === 'hive' && attentionCount > 0 ? (
                <View style={[styles.badge, { backgroundColor: t.userMessage }]}>
                  <Text style={styles.badgeText}>
                    {attentionCount > 9 ? '9+' : String(attentionCount)}
                  </Text>
                </View>
              ) : null}
            </Pressable>
          );
        })}
      </View>
      <Pressable
        onPress={onOpenSettings}
        style={styles.settings}
        accessibilityLabel="Open settings"
      >
        <Settings size={18} color={t.mutedForeground} strokeWidth={1.8} />
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  rail: {
    width: 52,
    borderRightWidth: StyleSheet.hairlineWidth,
    paddingVertical: 10,
    justifyContent: 'space-between',
  },
  top: {
    gap: 6,
    paddingHorizontal: 8,
  },
  item: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'transparent',
    borderRadius: 10,
    height: 36,
    alignItems: 'center',
    justifyContent: 'center',
    position: 'relative',
  },
  settings: {
    height: 36,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: 4,
  },
  badge: {
    position: 'absolute',
    top: 2,
    right: 2,
    minWidth: 14,
    height: 14,
    borderRadius: 7,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 3,
  },
  badgeText: {
    color: '#0b1119',
    fontSize: 9,
    fontWeight: '800',
  },
});
