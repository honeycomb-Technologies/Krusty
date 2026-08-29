import { memo } from 'react';
import { StyleSheet, Text, View } from 'react-native';
import { Folder, GitBranch } from 'lucide-react-native';
import Svg, { Circle } from 'react-native-svg';

const GAUGE_SIZE = 28;
const GAUGE_TOP_GAP = 4;
const META_ROW_HEIGHT = 24;

export interface ChatBarMetaRowProps {
  isDesktop: boolean;
  gaugeTokens: number;
  gaugeRadius: number;
  gaugeStroke: number;
  gaugeCircumference: number;
  gaugeOffset: number;
  gaugeColor: string;
  mutedForeground: string;
  workspaceContext: { hasBranch: boolean; label: string } | null;
  currentModelLabel: string;
  thinkingLabel: string;
}

function ChatBarMetaRowComponent({
  isDesktop,
  gaugeTokens,
  gaugeRadius,
  gaugeStroke,
  gaugeCircumference,
  gaugeOffset,
  gaugeColor,
  mutedForeground,
  workspaceContext,
  currentModelLabel,
  thinkingLabel,
}: ChatBarMetaRowProps) {
  return (
    <View
      pointerEvents="none"
      style={[styles.metaRow, !isDesktop && styles.metaRowMobile]}
    >
      {isDesktop || workspaceContext ? (
        <View style={styles.metaLeft}>
          {isDesktop ? (
            <View style={styles.gaugeRing}>
              <Svg width={GAUGE_SIZE} height={GAUGE_SIZE}>
                <Circle
                  cx={GAUGE_SIZE / 2}
                  cy={GAUGE_SIZE / 2}
                  r={gaugeRadius}
                  stroke={`${mutedForeground}26`}
                  strokeWidth={gaugeStroke}
                  fill="none"
                />
                <Circle
                  cx={GAUGE_SIZE / 2}
                  cy={GAUGE_SIZE / 2}
                  r={gaugeRadius}
                  stroke={gaugeColor}
                  strokeWidth={gaugeStroke}
                  fill="none"
                  strokeDasharray={`${gaugeCircumference}`}
                  strokeDashoffset={gaugeOffset}
                  strokeLinecap="round"
                  rotation={-90}
                  origin={`${GAUGE_SIZE / 2}, ${GAUGE_SIZE / 2}`}
                />
              </Svg>
              <Text style={[styles.gaugeLabel, { color: mutedForeground }]}>
                {gaugeTokens >= 1000
                  ? `${(gaugeTokens / 1000).toFixed(0)}k`
                  : gaugeTokens}
              </Text>
            </View>
          ) : null}
          {workspaceContext ? (
            <View style={styles.metaWorkspace}>
              {workspaceContext.hasBranch ? (
                <GitBranch size={12} color={mutedForeground} strokeWidth={1.8} />
              ) : (
                <Folder size={12} color={mutedForeground} strokeWidth={1.8} />
              )}
              <Text
                style={[styles.metaWorkspaceText, { color: mutedForeground }]}
                numberOfLines={1}
              >
                {workspaceContext.label}
              </Text>
            </View>
          ) : null}
        </View>
      ) : null}
      <View style={styles.metaRight}>
        <Text
          style={[styles.metaModel, { color: mutedForeground }]}
          numberOfLines={1}
        >
          {currentModelLabel}
        </Text>
        <Text
          style={[styles.metaDivider, { color: mutedForeground }]}
          numberOfLines={1}
        >
          |
        </Text>
        <Text
          style={[styles.metaThinking, { color: mutedForeground }]}
          numberOfLines={1}
        >
          {thinkingLabel}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  metaRow: {
    height: META_ROW_HEIGHT + GAUGE_TOP_GAP,
    paddingTop: GAUGE_TOP_GAP,
    paddingHorizontal: 4,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  metaRowMobile: { paddingHorizontal: 26 },
  metaLeft: {
    flex: 1,
    maxWidth: '54%',
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  metaRight: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: 10,
  },
  metaWorkspace: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
  },
  metaWorkspaceText: {
    flex: 1,
    minWidth: 0,
    fontSize: 11,
    fontWeight: '600',
    letterSpacing: 0,
  },
  metaModel: {
    flexShrink: 1,
    minWidth: 0,
    fontSize: 11,
    fontWeight: '700',
    letterSpacing: 0,
    textAlign: 'right',
  },
  metaDivider: {
    flexShrink: 0,
    fontSize: 11,
    fontWeight: '600',
    letterSpacing: 0,
  },
  metaThinking: {
    flexShrink: 0,
    fontSize: 11,
    fontWeight: '700',
    letterSpacing: 0,
  },
  gaugeRing: {
    width: GAUGE_SIZE,
    height: GAUGE_SIZE,
    alignItems: 'center',
    justifyContent: 'center',
  },
  gaugeLabel: {
    position: 'absolute',
    fontSize: 7,
    fontWeight: '600',
    letterSpacing: 0,
  },
});

export const ChatBarMetaRow = memo(ChatBarMetaRowComponent);
