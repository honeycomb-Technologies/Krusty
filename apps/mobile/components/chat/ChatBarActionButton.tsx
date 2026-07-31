import { memo, useEffect } from 'react';
import { Platform, Pressable, StyleSheet, View } from 'react-native';
import { ArrowUp, Mic } from 'lucide-react-native';
import Animated, {
  interpolate,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
} from 'react-native-reanimated';

export interface ChatBarActionButtonProps {
  isStreaming: boolean;
  isRecording: boolean;
  canSend: boolean;
  foreground: string;
  mutedForeground: string;
  onPress: () => void;
  onLongPress?: () => void;
}

const WEB_ACTION_STYLE =
  Platform.OS === 'web'
    ? ({
        outlineStyle: 'none',
        outlineWidth: 0,
      } as any)
    : null;

function ChatBarActionButtonComponent({
  isStreaming,
  isRecording,
  canSend,
  foreground,
  mutedForeground,
  onPress,
  onLongPress,
}: ChatBarActionButtonProps) {
  const stopProgress = useSharedValue(isStreaming || isRecording ? 1 : 0);

  useEffect(() => {
    stopProgress.value = withSpring(isStreaming || isRecording ? 1 : 0, {
      damping: 18,
      stiffness: 240,
      mass: 0.7,
    });
  }, [isRecording, isStreaming, stopProgress]);

  const arrowStyle = useAnimatedStyle(() => ({
    opacity: 1 - stopProgress.value,
    transform: [
      { scale: interpolate(stopProgress.value, [0, 1], [1, 0.58]) },
      { rotate: `${interpolate(stopProgress.value, [0, 1], [0, 22])}deg` },
    ],
  }));
  const stopStyle = useAnimatedStyle(() => ({
    opacity: stopProgress.value,
    borderRadius: interpolate(stopProgress.value, [0, 1], [6, 2.5]),
    transform: [
      { scale: interpolate(stopProgress.value, [0, 1], [0.56, 1]) },
      { rotate: `${interpolate(stopProgress.value, [0, 1], [-18, 0])}deg` },
    ],
  }));
  const showVoiceAction = !canSend && !isStreaming && !isRecording;

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={
        isStreaming
          ? 'Stop response'
          : isRecording
            ? 'Stop recording'
            : canSend
              ? 'Send message'
              : 'Start voice input'
      }
      onPress={onPress}
      onLongPress={canSend ? onLongPress : undefined}
      delayLongPress={300}
      style={({ pressed }) => [
        styles.actionBtn,
        WEB_ACTION_STYLE,
        {
          backgroundColor: pressed ? mutedForeground + '14' : 'transparent',
          transform: [{ scale: pressed ? 0.94 : 1 }],
        },
      ]}
    >
      {showVoiceAction ? (
        <Mic size={18} color={mutedForeground} strokeWidth={1.8} />
      ) : (
        <View style={styles.morphStage}>
          <Animated.View style={[styles.glyphLayer, arrowStyle]}>
            <ArrowUp size={20} color={foreground} strokeWidth={2.35} />
          </Animated.View>
          <Animated.View
            style={[
              styles.stopGlyph,
              { backgroundColor: foreground },
              stopStyle,
            ]}
          />
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  actionBtn: {
    width: 36,
    height: 36,
    borderRadius: 11,
    justifyContent: 'center',
    alignItems: 'center',
  },
  morphStage: {
    width: 22,
    height: 22,
    alignItems: 'center',
    justifyContent: 'center',
  },
  glyphLayer: {
    ...StyleSheet.absoluteFillObject,
    alignItems: 'center',
    justifyContent: 'center',
  },
  stopGlyph: {
    position: 'absolute',
    width: 11,
    height: 11,
  },
});

export const ChatBarActionButton = memo(ChatBarActionButtonComponent);
