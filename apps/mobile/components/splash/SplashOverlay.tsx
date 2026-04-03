import { useEffect, useState, useCallback } from 'react';
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

// Exact same text + styles as KrustyLogo
const LINES = [
  '▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌',
  '█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌',
  '▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪',
  '▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.',
  '·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ',
];

const K_CHARS = 6;
const K_LINES = LINES.map(l => l.slice(0, K_CHARS));

const GRADIENT_COLORS = [
  '#8b4513', '#cd853f', '#ff6b35', '#ffcc00',
  '#ff6b35', '#cd853f', '#8b4513',
] as const;

// Same as KrustyLogo
const GRADIENT_W = 500;
const GRADIENT_H = 80;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

/**
 * Animation flow:
 * 1. K appears CENTERED on screen (same Y as KrustyLogo empty state)
 * 2. K slides left to its final position (first letter of KRUSTY)
 * 3. A clip on the full KRUSTY text widens from K-width to full-width
 *    (the "unfold" — remaining letters appear left-to-right)
 * 4. Overlay fades out — KrustyLogo underneath is at exact same position,
 *    so logo stays while the rest of the UI (top bar, chat bar) appears
 *
 * Key insight: we use ONE MaskedView with the FULL text at all times,
 * clipped by an animated width. The whole thing translates from center
 * to its final left-aligned position. This way the gradient, text size,
 * and rendering match KrustyLogo exactly.
 */
export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // Measure real text widths via onLayout
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0;
  const restWidth = fullWidth - kWidth;

  // Clip width: starts at K-width, expands to full KRUSTY width
  const clipW = useSharedValue(0);
  // The whole text block translates: starts offset right (to center the K),
  // then slides to 0 (natural centered position of full text)
  const blockTranslateX = useSharedValue(0);

  // Shimmer
  const shimmer = useSharedValue(-80);

  // Overlay fades out
  const overlayOpacity = useSharedValue(1);

  const onKLayout = useCallback((e: { nativeEvent: { layout: { width: number } } }) => {
    if (kWidth === 0) setKWidth(e.nativeEvent.layout.width);
  }, [kWidth]);

  const onFullLayout = useCallback((e: { nativeEvent: { layout: { width: number } } }) => {
    if (fullWidth === 0) setFullWidth(e.nativeEvent.layout.width);
  }, [fullWidth]);

  useEffect(() => {
    if (!measured) return;

    // Initial state: clip to K-width, offset right so K appears centered
    // The full text is centered by the container. If we clip to K-width,
    // the visible K would be at the left edge of where KRUSTY would be.
    // To center just the K, we offset right by (fullWidth - kWidth) / 2.
    const centerOffset = restWidth / 2;
    clipW.value = kWidth;
    blockTranslateX.value = centerOffset;

    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Shimmer
      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold the centered K
      await new Promise(r => setTimeout(r, 600));

      // Slide left (offset → 0) while widening clip (K → full)
      blockTranslateX.value = withTiming(0, {
        duration: 700,
        easing: Easing.inOut(Easing.cubic),
      });

      clipW.value = withDelay(150, withTiming(fullWidth, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold
      await new Promise(r => setTimeout(r, 1100));

      // Fade overlay out
      overlayOpacity.value = withTiming(0, {
        duration: 400,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(finish)();
      });
    };

    run();
  }, [measured]);

  const clipStyle = useAnimatedStyle(() => ({
    width: clipW.value,
    overflow: 'hidden' as const,
  }));

  const translateStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: blockTranslateX.value }],
  }));

  const shimmerStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: shimmer.value }],
  }));

  const fadeStyle = useAnimatedStyle(() => ({
    opacity: overlayOpacity.value,
  }));

  if (done) return <>{children}</>;

  return (
    <View style={styles.root}>
      {/* App underneath */}
      <View style={StyleSheet.absoluteFill}>
        {children}
      </View>

      {/* Hidden measurement */}
      <View style={styles.measureContainer} pointerEvents="none">
        <View onLayout={onKLayout}>
          {K_LINES.map((line, i) => (
            <Text key={`mk${i}`} style={styles.line}>{line}</Text>
          ))}
        </View>
        <View onLayout={onFullLayout}>
          {LINES.map((line, i) => (
            <Text key={`mf${i}`} style={styles.line}>{line}</Text>
          ))}
        </View>
      </View>

      {/* Splash overlay */}
      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        {/* Same layout as ChatScreen empty state: flex-start, paddingTop 35%, center */}
        <View style={styles.logoPosition}>
          {/* Translate: centers the K initially, animates to 0 for final position */}
          <Animated.View style={translateStyle}>
            {/* Clip: starts at K-width, expands to full KRUSTY width */}
            <Animated.View style={clipStyle}>
              {/* Single MaskedView with FULL text — identical to KrustyLogo */}
              <MaskedView
                maskElement={
                  <View style={styles.maskAlign}>
                    {LINES.map((line, i) => (
                      <Text key={i} style={styles.line}>{line}</Text>
                    ))}
                  </View>
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
          </Animated.View>
        </View>
      </Animated.View>
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
  // Matches ChatScreen's empty state layout exactly
  logoPosition: {
    flex: 1,
    justifyContent: 'flex-start',
    alignItems: 'center',
    paddingTop: '35%',
  },
  maskAlign: {
    alignItems: 'center',
  },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
  },
  gradientWrap: {
    width: GRADIENT_W,
    height: GRADIENT_H,
  },
  gradient: {
    width: '100%',
    height: '100%',
  },
  measureContainer: {
    position: 'absolute',
    top: -9999,
    left: -9999,
    opacity: 0,
  },
});
