import { View, Text, Pressable, StyleSheet } from 'react-native';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';

interface SegmentControlProps {
  segments: string[];
  selected: number;
  onSelect: (index: number) => void;
}

export function SegmentControl({ segments, selected, onSelect }: SegmentControlProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.container, { backgroundColor: t.glass.background, borderColor: t.glass.border }]}>
      {segments.map((label, i) => {
        const active = i === selected;
        return (
          <Pressable
            key={label}
            onPress={() => {
              if (i !== selected) {
                Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onSelect(i);
              }
            }}
            style={[
              styles.segment,
              active && { backgroundColor: t.glass.backgroundElevated },
            ]}
          >
            <Text style={[
              styles.label,
              { color: active ? t.foreground : t.mutedForeground },
              active && styles.labelActive,
            ]}>
              {label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 2,
  },
  segment: {
    flex: 1,
    paddingVertical: 6,
    borderRadius: 8,
    alignItems: 'center',
  },
  label: {
    fontSize: 14,
    fontWeight: '500',
  },
  labelActive: {
    fontWeight: '600',
  },
});
