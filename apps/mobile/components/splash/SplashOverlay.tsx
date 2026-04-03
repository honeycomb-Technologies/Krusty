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

// Same text as KrustyLogo — must stay in sync
const LINES = [
  '▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌',
  '█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌',
  '▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪',
  '▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.',
  '·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ • ',
];

const K_CHARS = 6;
const K_LINES = LINES.map(l => l.slice(0, K_CHARS));
const REST_LINES = LINES.map(l => l.slice(K_CHARS));

// Same gradient as KrustyLogo
const GRADIENT_COLORS = [
  '#8b4513', '#cd853f', '#ff6b35', '#ffcc00',
  '#ff6b35', '#cd853f', '#8b4513',
] as const;

// Same gradient wrap size as KrustyLogo (500x80)
const GRADIENT_W = 500;
const GRADIENT_H = 80;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // Measure actual text widths on layout instead of guessing CHAR_W
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0;
  const restWidth = fullWidth - kWidth;
  const slideX = restWidth / 2;

  // K slides left from center
  const kTranslateX = useSharedValue(0);

  // RUSTY unfolds from K (animated width)
  const restClipW = useSharedValue(0);
  const restOpacity = useSharedValue(0);

  // Shimmer across gradient
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

    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Shimmer across the K
      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold centered K
      await new Promise(r => setTimeout(r, 600));

      // K slides left to make room for RUSTY
      kTranslateX.value = withTiming(-slideX, {
        duration: 600,
        easing: Easing.inOut(Easing.cubic),
      });

      // RUSTY unfolds from K's right edge
      restOpacity.value = withDelay(200, withTiming(1, { duration: 300 }));
      restClipW.value = withDelay(200, withTiming(restWidth, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold to complete
      await new Promise(r => setTimeout(r, 1100));

      // Fade out overlay — KrustyLogo underneath is at the same position,
      // so the logo appears to stay while UI fades in around it
      overlayOpacity.value = withTiming(0, {
        duration: 400,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(finish)();
      });
    };

    run();
  }, [measured]);

  const kSlideStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: kTranslateX.value }],
  }));

  const restClipStyle = useAnimatedStyle(() => ({
    width: restClipW.value,
    opacity: restOpacity.value,
    overflow: 'hidden' as const,
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
      {/* App content underneath — visible as overlay fades */}
      <View style={StyleSheet.absoluteFill}>
        {children}
      </View>

      {/* Hidden measurement views — render full text to get real widths */}
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

      {/* Splash overlay — matches KrustyLogo's position (paddingTop 35%, flex-start, centered) */}
      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        <View style={styles.logoPosition}>
          <Animated.View style={[styles.logoRow, kSlideStyle]}>
            {/* K portion — uses same MaskedView + gradient pattern as KrustyLogo */}
            <MaskedView
              maskElement={
                <View style={styles.maskAlign}>
                  {K_LINES.map((line, i) => (
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

            {/* RUSTY — unfolds from K's right edge */}
            <Animated.View style={restClipStyle}>
              <MaskedView
                maskElement={
                  <View style={styles.maskAlign}>
                    {REST_LINES.map((line, i) => (
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
  // Match the empty state layout in ChatScreen:
  // flex: 1, justifyContent: 'flex-start', alignItems: 'center', paddingTop: '35%'
  logoPosition: {
    flex: 1,
    justifyContent: 'flex-start',
    alignItems: 'center',
    paddingTop: '35%',
  },
  logoRow: {
    flexDirection: 'row',
    alignItems: 'flex-start',
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
  // Same size as KrustyLogo's gradientWrap (500x80)
  gradientWrap: {
    width: GRADIENT_W,
    height: GRADIENT_H,
  },
  gradient: {
    width: '100%',
    height: '100%',
  },
  // Off-screen container to measure real text widths
  measureContainer: {
    position: 'absolute',
    top: -9999,
    left: -9999,
    opacity: 0,
  },
});
