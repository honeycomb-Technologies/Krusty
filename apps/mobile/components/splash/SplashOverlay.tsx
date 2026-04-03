import { useEffect, useState } from 'react';
import { View, Text, StyleSheet } from 'react-native';
import MaskedView from '@react-native-masked-view/masked-view';
import { LinearGradient } from '../../platform/linear-gradient';
import * as SplashScreen from 'expo-splash-screen';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withTiming,
  withDelay,
  Easing,
  runOnJS,
} from 'react-native-reanimated';

SplashScreen.preventAutoHideAsync();

const BG = '#0b1119';

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

const CHAR_W = 7.2;
const K_CHARS = 6;
const FULL_WIDTH = LINES[0].length * CHAR_W;
const K_WIDTH = K_CHARS * CHAR_W;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [phase, setPhase] = useState<'animate' | 'reveal' | 'done'>('animate');

  // Phase 1: K → KRUSTY text unravel
  const textClip = useSharedValue(K_WIDTH);
  const logoScale = useSharedValue(2.2);
  const shimmer = useSharedValue(-100);

  // Phase 2: Mask expand (Twitter/X style)
  const maskScale = useSharedValue(1);
  const appScale = useSharedValue(1.05);
  const appOpacity = useSharedValue(0);

  useEffect(() => {
    const setDone = () => {
      setPhase('done');
      onComplete?.();
    };

    const run = async () => {
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      shimmer.value = withTiming(100, {
        duration: 2000,
        easing: Easing.inOut(Easing.ease),
      });

      await new Promise(r => setTimeout(r, 400));

      logoScale.value = withTiming(1, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      });

      textClip.value = withTiming(FULL_WIDTH, {
        duration: 900,
        easing: Easing.out(Easing.cubic),
      });

      // After text fully revealed, switch to mask reveal
      await new Promise(r => setTimeout(r, 1100));
      setPhase('reveal');

      appOpacity.value = withDelay(150, withTiming(1, { duration: 350 }));
      appScale.value = withDelay(150, withTiming(1, {
        duration: 500,
        easing: Easing.out(Easing.cubic),
      }));

      maskScale.value = withTiming(70, {
        duration: 800,
        easing: Easing.in(Easing.cubic),
      }, () => {
        runOnJS(setDone)();
      });
    };

    run();
  }, []);

  const clipStyle = useAnimatedStyle(() => ({
    width: textClip.value,
    overflow: 'hidden' as const,
  }));

  const scaleStyle = useAnimatedStyle(() => ({
    transform: [{ scale: logoScale.value }],
  }));

  const shimmerStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: shimmer.value }],
  }));

  const maskExpandStyle = useAnimatedStyle(() => ({
    transform: [{ scale: maskScale.value }],
  }));

  const appRevealStyle = useAnimatedStyle(() => ({
    opacity: appOpacity.value,
    transform: [{ scale: appScale.value }],
  }));

  if (phase === 'done') return <>{children}</>;

  if (phase === 'reveal') {
    // Twitter/X-style reveal:
    // Parent View has BG color (the "overlay").
    // MaskedView uses KRUSTY logo as mask — app content shows ONLY through the logo shape.
    // As the logo scales to 70x, the visible "window" grows until the entire screen is app.
    return (
      <View style={[styles.root, { backgroundColor: BG }]}>
        <MaskedView
          style={StyleSheet.absoluteFill}
          maskElement={
            <View style={styles.center}>
              <Animated.View style={maskExpandStyle}>
                <View style={styles.logoBlock}>
                  {LINES.map((line, i) => (
                    <Text key={i} style={styles.maskText}>{line}</Text>
                  ))}
                </View>
              </Animated.View>
            </View>
          }
        >
          <Animated.View style={[StyleSheet.absoluteFill, appRevealStyle]}>
            {children}
          </Animated.View>
        </MaskedView>
      </View>
    );
  }

  // Phase 'animate': Solid BG overlay with K → KRUSTY text animation
  return (
    <View style={styles.root}>
      <View style={StyleSheet.absoluteFill}>
        {children}
      </View>
      <View style={styles.solidOverlay} pointerEvents="none">
        <View style={styles.center}>
          <Animated.View style={scaleStyle}>
            <MaskedView
              maskElement={
                <Animated.View style={clipStyle}>
                  <View style={styles.textBlock}>
                    {LINES.map((line, i) => (
                      <Text key={i} style={styles.line}>{line}</Text>
                    ))}
                  </View>
                </Animated.View>
              }
            >
              <Animated.View style={[styles.gradientWrap, shimmerStyle]}>
                <LinearGradient
                  colors={[...GRADIENT_COLORS]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={styles.gradient}
                />
              </Animated.View>
            </MaskedView>
          </Animated.View>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
  },
  solidOverlay: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: BG,
    zIndex: 10,
  },
  center: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    backgroundColor: 'transparent',
  },
  textBlock: {
    alignItems: 'flex-start',
  },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
  },
  gradientWrap: {
    width: 500,
    height: 80,
  },
  gradient: {
    width: '100%',
    height: '100%',
  },
  logoBlock: {
    alignItems: 'center',
  },
  maskText: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: 'black',
  },
});
