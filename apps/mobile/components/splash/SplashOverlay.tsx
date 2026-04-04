import { useEffect, useState, useCallback } from 'react';
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

const BG = '#0b1119';
const SCREEN_H = Dimensions.get('window').height;
const SCREEN_W = Dimensions.get('window').width;

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

const PADDING_TOP = SCREEN_W * 0.35;
const DROP_Y = (SCREEN_H / 2 - 40) - PADDING_TOP;

type Phase = 'zoom' | 'unfold' | 'fade' | 'done';

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [phase, setPhase] = useState<Phase>('zoom');
  const [done, setDone] = useState(false);

  // Measure
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const [textHeight, setTextHeight] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0 && textHeight > 0;
  const restWidth = fullWidth - kWidth;

  // Phase 1: Zoom — only K visible, zoomed in at center
  const zoomScale = useSharedValue(2.5);
  const zoomY = useSharedValue(DROP_Y);
  const shimmer = useSharedValue(-80);

  // Phase 2: Unfold — full text, K slides left, cover reveals RUSTY
  const unfoldX = useSharedValue(0);
  const coverX = useSharedValue(0);

  // Phase 3: Fade
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

    unfoldX.value = restWidth / 2;
    coverX.value = 0;

    const run = async () => {
      // === Phase 0: Hide native splash ===
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();
      shimmer.value = withTiming(80, { duration: 3000, easing: Easing.inOut(Easing.ease) });

      // === Phase 1: Hold zoomed K at screen center (500ms) ===
      await new Promise(r => setTimeout(r, 500));

      // === Phase 2: Zoom out K to final size + position (600ms) ===
      zoomScale.value = withTiming(1.0, { duration: 600, easing: Easing.out(Easing.cubic) });
      zoomY.value = withTiming(0, { duration: 600, easing: Easing.out(Easing.cubic) });

      // Wait for zoom to FULLY complete
      await new Promise(r => setTimeout(r, 700));

      // === Switch to unfold phase ===
      // Now show full text with cover instead of just K
      setPhase('unfold');

      // Small delay to let the phase switch render
      await new Promise(r => setTimeout(r, 50));

      // === Phase 3: K slides left + RUSTY revealed (600ms) ===
      unfoldX.value = withTiming(0, { duration: 600, easing: Easing.inOut(Easing.cubic) });
      coverX.value = withDelay(100, withTiming(restWidth + 50, {
        duration: 600,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold to complete
      await new Promise(r => setTimeout(r, 800));

      // === Phase 4: Fade out ===
      setPhase('fade');
      overlayOpacity.value = withTiming(0, {
        duration: 350,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(setDone)(true);
        if (onComplete) runOnJS(onComplete)();
      });
    };

    run();
  }, [measured]);

  // Zoom phase: K only, scaled and offset
  const zoomStyle = useAnimatedStyle(() => ({
    transform: [
      { translateY: zoomY.value },
      { scale: zoomScale.value },
    ],
  }));

  // Unfold phase: full text, offset right then sliding to 0
  const unfoldStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: unfoldX.value }],
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
      <View style={StyleSheet.absoluteFill}>{children}</View>

      {/* Measure */}
      <View style={styles.measureContainer} pointerEvents="none">
        <View onLayout={onKLayout}>
          {K_LINES.map((line, i) => <Text key={`mk${i}`} style={styles.line}>{line}</Text>)}
        </View>
        <View onLayout={onFullLayout}>
          {LINES.map((line, i) => <Text key={`mf${i}`} style={styles.line}>{line}</Text>)}
        </View>
      </View>

      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        <View style={styles.logoPosition}>

          {/* ZOOM PHASE: Only the K, centered, scaled */}
          {phase === 'zoom' && (
            <Animated.View style={zoomStyle}>
              <MaskedView
                maskElement={
                  <View>
                    {K_LINES.map((line, i) => <Text key={i} style={styles.line}>{line}</Text>)}
                  </View>
                }
              >
                <Animated.View style={[{ width: kWidth || 200, height: textHeight || 80 }, shimmerStyle]}>
                  <LinearGradient
                    colors={[...GRADIENT_COLORS]}
                    start={{ x: 0, y: 0 }}
                    end={{ x: 1, y: 0 }}
                    style={styles.gradient}
                  />
                </Animated.View>
              </MaskedView>
            </Animated.View>
          )}

          {/* UNFOLD + FADE PHASES: Full text with cover */}
          {(phase === 'unfold' || phase === 'fade') && (
            <Animated.View style={unfoldStyle}>
              <View style={styles.textContainer}>
                <MaskedView
                  maskElement={
                    <View>
                      {LINES.map((line, i) => <Text key={i} style={styles.line}>{line}</Text>)}
                    </View>
                  }
                >
                  <Animated.View style={[{ width: fullWidth || 500, height: textHeight || 80 }, shimmerStyle]}>
                    <LinearGradient
                      colors={[...GRADIENT_COLORS]}
                      start={{ x: 0, y: 0 }}
                      end={{ x: 1, y: 0 }}
                      style={styles.gradient}
                    />
                  </Animated.View>
                </MaskedView>

                {/* Cover hides RUSTY, slides right to reveal */}
                <Animated.View
                  style={[
                    {
                      position: 'absolute',
                      top: -5,
                      left: kWidth,
                      width: restWidth + 60,
                      height: textHeight + 10,
                      backgroundColor: BG,
                    },
                    coverStyle,
                  ]}
                />
              </View>
            </Animated.View>
          )}

        </View>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
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
  textContainer: { position: 'relative' },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
  },
  gradient: { width: '100%', height: '100%' },
  measureContainer: {
    position: 'absolute',
    top: -9999,
    left: -9999,
    opacity: 0,
  },
});
