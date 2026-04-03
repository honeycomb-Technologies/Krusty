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

// Identical to KrustyLogo
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

const GRADIENT_W = 500;
const GRADIENT_H = 80;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

/**
 * Approach: the full KRUSTY text is ALWAYS rendered identically to KrustyLogo
 * (same MaskedView, same gradient, same alignment). It's never clipped or split.
 *
 * To create the animation:
 * 1. A BG-colored "cover" rectangle sits ON TOP of the text, hiding everything
 *    to the right of the K. The whole assembly is shifted right so the visible
 *    K appears centered on screen.
 * 2. The assembly slides left to its final position while the cover slides
 *    right (revealing RUSTY) — the letters appear to emerge from behind the K.
 * 3. The solid overlay fades out, revealing the app with KrustyLogo in the
 *    exact same position underneath.
 */
export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // Measure the K portion to know where the cover starts
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const [textHeight, setTextHeight] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0 && textHeight > 0;

  // The whole text block shifts right (to center K) then back to 0
  const blockX = useSharedValue(0);
  // Drop: K starts higher (50% of screen) and snaps down to 35%
  // The container already has paddingTop 35%, so we offset up by ~15% of screen height
  const dropY = useSharedValue(0);
  // Quick scale pulse when K lands
  const landScale = useSharedValue(1);
  // The cover slides right to reveal RUSTY
  const coverX = useSharedValue(0);
  // Shimmer
  const shimmer = useSharedValue(-80);
  // Overlay fades out at the end
  const overlayOpacity = useSharedValue(1);

  const onKLayout = useCallback((e: { nativeEvent: { layout: { width: number } } }) => {
    if (kWidth === 0) setKWidth(e.nativeEvent.layout.width);
  }, [kWidth]);

  const onFullLayout = useCallback((e: { nativeEvent: { layout: { width: number; height: number } } }) => {
    if (fullWidth === 0) {
      setFullWidth(e.nativeEvent.layout.width);
      setTextHeight(e.nativeEvent.layout.height);
    }
  }, [fullWidth]);

  useEffect(() => {
    if (!measured) return;

    const restWidth = fullWidth - kWidth;
    // Offset to center just the K: shift right by half the RUSTY width
    const centerOffset = restWidth / 2;

    // Set initial positions
    blockX.value = centerOffset;
    coverX.value = 0;
    // Start K at ~50% screen (offset up from the 35% paddingTop position)
    // 15% of screen height ≈ the difference between 50% and 35%
    dropY.value = -120;
    landScale.value = 1.05;

    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold at 50% briefly
      await new Promise(r => setTimeout(r, 400));

      // Quick snap down to 35% with scale pulse
      dropY.value = withTiming(0, {
        duration: 250,
        easing: Easing.in(Easing.cubic),
      });
      landScale.value = withDelay(200, withTiming(1, {
        duration: 150,
        easing: Easing.out(Easing.cubic),
      }));

      // Brief hold after landing
      await new Promise(r => setTimeout(r, 450));

      // Slide the whole block left to final position
      blockX.value = withTiming(0, {
        duration: 700,
        easing: Easing.inOut(Easing.cubic),
      });

      // Slide the cover right to reveal RUSTY (emerging from behind K)
      coverX.value = withDelay(100, withTiming(restWidth + 20, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for animation to settle
      await new Promise(r => setTimeout(r, 1000));

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

  const blockStyle = useAnimatedStyle(() => ({
    transform: [
      { translateX: blockX.value },
      { translateY: dropY.value },
      { scale: landScale.value },
    ],
  }));

  const coverStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: coverX.value }],
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
        <View style={styles.logoPosition}>
          {/* Block translates from centered-K position to final position */}
          <Animated.View style={blockStyle}>
            <View style={{ position: 'relative' }}>
              {/* Full KRUSTY text — identical rendering to KrustyLogo */}
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

              {/* Cover rectangle — hides RUSTY, slides right to reveal */}
              <Animated.View
                style={[
                  {
                    position: 'absolute',
                    top: -5,
                    left: kWidth,
                    width: fullWidth,
                    height: textHeight + 10,
                    backgroundColor: BG,
                  },
                  coverStyle,
                ]}
              />
            </View>
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
