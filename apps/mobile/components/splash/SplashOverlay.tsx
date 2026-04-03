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
  withSequence,
  Easing,
  runOnJS,
} from 'react-native-reanimated';

SplashScreen.preventAutoHideAsync();

const BG = '#0b1119';
const SCREEN_H = Dimensions.get('window').height;
const SCREEN_W = Dimensions.get('window').width;

// Same text as KrustyLogo
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

// RN paddingTop '35%' = 35% of container WIDTH
const PADDING_TOP = SCREEN_W * 0.35;
// Distance from 35% position down to screen center
const DROP_DISTANCE = (SCREEN_H / 2 - 40) - PADDING_TOP;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // Measure text widths with flex-start alignment (no centering offset)
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const [textHeight, setTextHeight] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0 && textHeight > 0;
  const restWidth = fullWidth - kWidth;

  // ---- Animated values ----
  // Phase 1: K at screen center
  const posY = useSharedValue(DROP_DISTANCE);    // positive = below 35%, at screen center
  const posScale = useSharedValue(1.08);

  // Phase 2: Unfold — K slides left, cover reveals RUSTY
  const offsetX = useSharedValue(0);             // shifts whole block right to center K
  const coverX = useSharedValue(0);              // cover slides right

  // Ambient
  const shimmer = useSharedValue(-80);
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

    // Initial state: K centered horizontally and at screen center vertically
    offsetX.value = restWidth / 2;
    posY.value = DROP_DISTANCE;
    posScale.value = 1.08;
    coverX.value = 0;

    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      // ===== PHASE 0: Hide native splash (300ms) =====
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Start shimmer (runs throughout)
      shimmer.value = withTiming(80, { duration: 2500, easing: Easing.inOut(Easing.ease) });

      // ===== PHASE 1: Hold K at screen center (500ms) =====
      await new Promise(r => setTimeout(r, 500));

      // ===== PHASE 2: Quick snap up to 35% with scale pulse (350ms total) =====
      posY.value = withTiming(0, { duration: 250, easing: Easing.out(Easing.cubic) });
      posScale.value = withSequence(
        withTiming(0.97, { duration: 200, easing: Easing.in(Easing.cubic) }),
        withTiming(1.0, { duration: 150, easing: Easing.out(Easing.cubic) }),
      );

      // Wait for snap to fully complete before unfold
      await new Promise(r => setTimeout(r, 500));

      // ===== PHASE 3: K slides left + RUSTY unfolds (700ms) =====
      offsetX.value = withTiming(0, { duration: 600, easing: Easing.inOut(Easing.cubic) });
      // Cover starts sliding 150ms after K starts moving
      coverX.value = withDelay(150, withTiming(restWidth + 50, {
        duration: 600,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold to fully complete
      await new Promise(r => setTimeout(r, 900));

      // ===== PHASE 4: Fade out overlay (350ms) =====
      overlayOpacity.value = withTiming(0, {
        duration: 350,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(finish)();
      });
    };

    run();
  }, [measured]);

  // ---- Animated styles ----
  const blockStyle = useAnimatedStyle(() => ({
    transform: [
      { translateX: offsetX.value },
      { translateY: posY.value },
      { scale: posScale.value },
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
      {/* App content underneath */}
      <View style={StyleSheet.absoluteFill}>{children}</View>

      {/* Hidden measurement — uses flex-start so widths are accurate */}
      <View style={styles.measureContainer} pointerEvents="none">
        <View onLayout={onKLayout} style={styles.measureRow}>
          {K_LINES.map((line, i) => (
            <Text key={`mk${i}`} style={styles.line}>{line}</Text>
          ))}
        </View>
        <View onLayout={onFullLayout} style={styles.measureRow}>
          {LINES.map((line, i) => (
            <Text key={`mf${i}`} style={styles.line}>{line}</Text>
          ))}
        </View>
      </View>

      {/* Splash overlay */}
      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        {/* Position matches ChatScreen empty state: paddingTop 35%, center, flex-start */}
        <View style={styles.logoPosition}>
          <Animated.View style={blockStyle}>
            <View style={styles.textContainer}>
              {/*
                MaskedView renders the full KRUSTY text.
                The mask uses flex-start alignment so text starts at x=0.
                The gradient is positioned to cover just the text width.
                This means cover left:kWidth correctly sits at the K's right edge.
              */}
              <MaskedView
                maskElement={
                  <View>
                    {LINES.map((line, i) => (
                      <Text key={i} style={styles.line}>{line}</Text>
                    ))}
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

              {/* Cover — same color as BG, hides RUSTY, slides right to reveal */}
              {measured && (
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
              )}
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
  textContainer: {
    position: 'relative',
  },
  line: {
    fontFamily: 'Courier',
    fontSize: 12,
    lineHeight: 14,
    letterSpacing: 0,
    color: '#000',
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
  measureRow: {
    alignItems: 'flex-start',
  },
});
