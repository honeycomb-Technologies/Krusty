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

// paddingTop '35%' in RN = 35% of container WIDTH
const LOGO_PADDING_TOP = SCREEN_W * 0.35;
// Where 50% of screen is, relative to the logo's resting position
const CENTER_OFFSET_Y = (SCREEN_H / 2 - 40) - LOGO_PADDING_TOP; // -40 for text height center

// Same text/gradient as KrustyLogo — MUST stay in sync
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

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // Measure real text widths
  const [kWidth, setKWidth] = useState(0);
  const [fullWidth, setFullWidth] = useState(0);
  const [textHeight, setTextHeight] = useState(0);
  const measured = kWidth > 0 && fullWidth > 0 && textHeight > 0;

  // Animated values
  const translateX = useSharedValue(0);  // horizontal: centers K, then slides to 0
  const translateY = useSharedValue(CENTER_OFFSET_Y); // vertical: starts at screen center, snaps to 0 (35%)
  const scale = useSharedValue(1.08);    // slight scale at center, settles to 1
  const coverX = useSharedValue(0);      // cover slides right to reveal RUSTY
  const shimmer = useSharedValue(-80);   // gradient shimmer
  const overlayOpacity = useSharedValue(1); // fades out at end

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
    const centerOffsetX = restWidth / 2;

    // Initial state: K centered on screen
    translateX.value = centerOffsetX; // shift right so K (not full text) is centered
    translateY.value = CENTER_OFFSET_Y; // positive = further down = screen center
    scale.value = 1.08;
    coverX.value = 0;

    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      // Hide native splash — our overlay matches it
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Shimmer across K while holding at center
      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold at screen center
      await new Promise(r => setTimeout(r, 500));

      // SNAP: K moves up from 50% to 35%, scale pulses 1.08 → 0.97 → 1.0
      translateY.value = withTiming(0, {
        duration: 250,
        easing: Easing.out(Easing.cubic),
      });
      scale.value = withSequence(
        withTiming(0.97, { duration: 200, easing: Easing.in(Easing.cubic) }),
        withTiming(1.0, { duration: 150, easing: Easing.out(Easing.cubic) }),
      );

      // Brief settle
      await new Promise(r => setTimeout(r, 400));

      // UNFOLD: K slides left, cover reveals RUSTY
      translateX.value = withTiming(0, {
        duration: 600,
        easing: Easing.inOut(Easing.cubic),
      });
      coverX.value = withDelay(100, withTiming(restWidth + 20, {
        duration: 600,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold
      await new Promise(r => setTimeout(r, 800));

      // FADE: overlay disappears, app underneath (with KrustyLogo) is revealed
      overlayOpacity.value = withTiming(0, {
        duration: 350,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(finish)();
      });
    };

    run();
  }, [measured]);

  const blockStyle = useAnimatedStyle(() => ({
    transform: [
      { translateX: translateX.value },
      { translateY: translateY.value },
      { scale: scale.value },
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
      <View style={StyleSheet.absoluteFill}>
        {children}
      </View>

      {/* Hidden measurement views */}
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

      {/* Splash overlay — solid BG with animated logo */}
      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        {/* logoPosition matches ChatScreen empty state exactly */}
        <View style={styles.logoPosition}>
          <Animated.View style={blockStyle}>
            <View style={styles.relative}>
              {/* Full KRUSTY text — single MaskedView, same as KrustyLogo */}
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

              {/* Cover — BG-colored rect hiding RUSTY, slides right to reveal */}
              {kWidth > 0 && (
                <Animated.View
                  style={[
                    {
                      position: 'absolute',
                      top: -5,
                      left: kWidth,
                      width: fullWidth + 50,
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
  // Must match ChatScreen's empty state: flex-start, center, paddingTop 35%
  logoPosition: {
    flex: 1,
    justifyContent: 'flex-start',
    alignItems: 'center',
    paddingTop: '35%',
  },
  relative: {
    position: 'relative',
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
    width: 500,
    height: 80,
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
