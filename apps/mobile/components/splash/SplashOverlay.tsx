import { useEffect, useState } from 'react';
import { View, Text, StyleSheet, Dimensions } from 'react-native';
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

const { width: SCREEN_W } = Dimensions.get('window');
const BG = '#0b1119';

// Full KRUSTY ASCII art
const LINES = [
  '▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌',
  '█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌',
  '▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪',
  '▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.',
  '·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ',
];

// Split: K portion (first 6 chars) vs the rest
const K_CHARS = 6;
const K_LINES = LINES.map(l => l.slice(0, K_CHARS));
const REST_LINES = LINES.map(l => l.slice(K_CHARS));

const GRADIENT_COLORS = [
  '#8b4513', '#cd853f', '#ff6b35', '#ffcc00',
  '#ff6b35', '#cd853f', '#8b4513',
] as const;

const CHAR_W = 7.2;
const FULL_TEXT_W = LINES[0].length * CHAR_W;
const K_TEXT_W = K_CHARS * CHAR_W;
const REST_TEXT_W = FULL_TEXT_W - K_TEXT_W;

// How far the K needs to slide left from center to its final position
// In the final layout, the full text is centered, so K starts at center
// and needs to move left by half the rest-of-text width
const K_SLIDE_X = REST_TEXT_W / 2;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [phase, setPhase] = useState<'k-center' | 'unfold' | 'reveal' | 'done'>('k-center');

  // K starts centered, then slides left
  const kTranslateX = useSharedValue(0);

  // Rest of letters clip from 0 → full width (unfold from K)
  const restClipW = useSharedValue(0);
  // Rest opacity fades in
  const restOpacity = useSharedValue(0);

  // Shimmer across the gradient
  const shimmer = useSharedValue(-80);

  // Mask expand (Twitter/X reveal)
  const maskScale = useSharedValue(1);
  const appScale = useSharedValue(1.05);
  const appOpacity = useSharedValue(0);

  useEffect(() => {
    const setDone = () => {
      setPhase('done');
      onComplete?.();
    };

    const run = async () => {
      // Brief pause, then hide native splash (our overlay matches it)
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Start shimmer on the K
      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold the centered K for a moment
      await new Promise(r => setTimeout(r, 600));

      // Phase 2: K slides left, letters unfold
      setPhase('unfold');

      kTranslateX.value = withTiming(-K_SLIDE_X, {
        duration: 600,
        easing: Easing.inOut(Easing.cubic),
      });

      // Letters unfold from K with a slight delay
      restOpacity.value = withDelay(200, withTiming(1, { duration: 300 }));
      restClipW.value = withDelay(200, withTiming(REST_TEXT_W, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold to complete, then mask reveal
      await new Promise(r => setTimeout(r, 1200));
      setPhase('reveal');

      appOpacity.value = withDelay(100, withTiming(1, { duration: 350 }));
      appScale.value = withDelay(100, withTiming(1, {
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

  // K slides left from center
  const kSlideStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: kTranslateX.value }],
  }));

  // Rest of letters clip-reveal from left edge
  const restClipStyle = useAnimatedStyle(() => ({
    width: restClipW.value,
    opacity: restOpacity.value,
    overflow: 'hidden' as const,
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

  // Phases 'k-center' and 'unfold':
  // K is centered, then slides left while rest unfolds from it
  return (
    <View style={styles.root}>
      <View style={StyleSheet.absoluteFill}>
        {children}
      </View>
      <View style={styles.solidOverlay} pointerEvents="none">
        <View style={styles.center}>
          {/* Container that holds K + rest in a row, animated as a group */}
          <Animated.View style={[styles.logoRow, kSlideStyle]}>
            {/* The K portion — always visible */}
            <MaskedView
              maskElement={
                <View>
                  {K_LINES.map((line, i) => (
                    <Text key={i} style={styles.line}>{line}</Text>
                  ))}
                </View>
              }
            >
              <Animated.View style={[styles.kGradientWrap, shimmerStyle]}>
                <LinearGradient
                  colors={[...GRADIENT_COLORS]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={styles.gradient}
                />
              </Animated.View>
            </MaskedView>

            {/* The rest (RUSTY) — clips open from left */}
            <Animated.View style={restClipStyle}>
              <MaskedView
                maskElement={
                  <View>
                    {REST_LINES.map((line, i) => (
                      <Text key={i} style={styles.line}>{line}</Text>
                    ))}
                  </View>
                }
              >
                <Animated.View style={[styles.restGradientWrap, shimmerStyle]}>
                  <LinearGradient
                    colors={[...GRADIENT_COLORS]}
                    start={{ x: 0, y: 0 }}
                    end={{ x: 1, y: 0 }}
                    style={styles.gradient}
                  />
                </Animated.View>
              </MaskedView>
            </Animated.View>
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
  logoRow: {
    flexDirection: 'row',
    alignItems: 'flex-start',
  },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
  },
  kGradientWrap: {
    width: 300,
    height: 80,
  },
  restGradientWrap: {
    width: 400,
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
