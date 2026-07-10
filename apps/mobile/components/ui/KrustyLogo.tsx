import { useEffect } from 'react';
import { Platform, View, Text, StyleSheet, type StyleProp, type TextStyle } from 'react-native';
import MaskedView from '@react-native-masked-view/masked-view';
import { LinearGradient } from '../../platform/linear-gradient';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withRepeat,
  withTiming,
  withDelay,
  Easing,
  interpolate,
} from 'react-native-reanimated';

const LINES = [
  '▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌',
  '█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌',
  '▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪',
  '▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.',
  '·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ',
];

const GRADIENT_COLORS = [
  '#8b4513', '#cd853f', '#ff6b35', '#ffcc00',
  '#ff6b35', '#cd853f', '#8b4513',
] as const;
const WEB_GRADIENT = `linear-gradient(90deg, ${GRADIENT_COLORS.join(', ')})`;
const WEB_KEYFRAME_ID = 'krusty-logo-gradient-keyframes';

const webGradientTextStyle = {
  backgroundImage: WEB_GRADIENT,
  backgroundSize: '220% 100%',
  backgroundPosition: '0% 50%',
  backgroundClip: 'text',
  WebkitBackgroundClip: 'text',
  color: 'transparent',
  WebkitTextFillColor: 'transparent',
  animationName: 'krusty-logo-gradient-shift',
  animationDuration: '4s',
  animationTimingFunction: 'ease-in-out',
  animationIterationCount: 'infinite',
  animationDirection: 'alternate',
} as const;

function useWebGradientKeyframes() {
  useEffect(() => {
    if (Platform.OS !== 'web' || typeof document === 'undefined') return;
    if (document.getElementById(WEB_KEYFRAME_ID)) return;

    const style = document.createElement('style');
    style.id = WEB_KEYFRAME_ID;
    style.textContent = `
      @keyframes krusty-logo-gradient-shift {
        from { background-position: 0% 50%; }
        to { background-position: 100% 50%; }
      }
    `;
    document.head.appendChild(style);
  }, []);
}

function WaveLine({
  text,
  index,
  textStyle,
}: {
  text: string;
  index: number;
  textStyle?: StyleProp<TextStyle>;
}) {
  const wave = useSharedValue(0);

  useEffect(() => {
    wave.value = withDelay(
      index * 100,
      withRepeat(
        withTiming(1, { duration: 3000, easing: Easing.inOut(Easing.ease) }),
        -1,
        true,
      ),
    );
  }, []);

  const animStyle = useAnimatedStyle(() => ({
    transform: [
      { translateY: interpolate(wave.value, [0, 0.25, 0.5, 0.75, 1], [0, -2, 0, 2, 0]) },
    ],
  }));

  return (
    <Animated.View style={animStyle}>
      <Text style={[styles.line, textStyle]}>{text}</Text>
    </Animated.View>
  );
}

export function KrustyLogo() {
  if (Platform.OS === 'web') {
    return <WebKrustyLogo />;
  }

  return <NativeKrustyLogo />;
}

function WebKrustyLogo() {
  useWebGradientKeyframes();

  return (
    <View style={styles.webLogo}>
      {LINES.map((line, i) => (
        <WaveLine
          key={line}
          text={line}
          index={i}
          textStyle={[styles.webLine, webGradientTextStyle as unknown as TextStyle]}
        />
      ))}
    </View>
  );
}

function NativeKrustyLogo() {
  const shimmer = useSharedValue(0);

  useEffect(() => {
    shimmer.value = withRepeat(
      withTiming(1, { duration: 4000, easing: Easing.inOut(Easing.ease) }),
      -1,
      true,
    );
  }, []);

  const gradientStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: interpolate(shimmer.value, [0, 1], [-80, 80]) }],
  }));

  return (
    <View style={styles.container}>
      <MaskedView
        maskElement={
          <View style={styles.mask}>
            {LINES.map((line, i) => (
              <WaveLine key={i} text={line} index={i} />
            ))}
          </View>
        }
      >
        <Animated.View style={[styles.gradientWrap, gradientStyle]}>
          <LinearGradient
            colors={[...GRADIENT_COLORS]}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            style={styles.gradient}
          />
        </Animated.View>
      </MaskedView>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    alignItems: 'center',
  },
  mask: {
    alignItems: 'center',
  },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
  },
  webLogo: {
    alignItems: 'center',
  },
  webLine: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
  },
  gradientWrap: {
    width: 500,
    height: 80,
  },
  gradient: {
    width: '100%',
    height: '100%',
  },
});
