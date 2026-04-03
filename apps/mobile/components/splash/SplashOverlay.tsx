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
const K_SLIDE_X = REST_TEXT_W / 2;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);

  // K slides left from center
  const kTranslateX = useSharedValue(0);

  // RUSTY unfolds from K
  const restClipW = useSharedValue(0);
  const restOpacity = useSharedValue(0);

  // Shimmer
  const shimmer = useSharedValue(-80);

  // Overlay fades out to reveal app UI around the logo
  const overlayOpacity = useSharedValue(1);

  useEffect(() => {
    const finish = () => {
      setDone(true);
      onComplete?.();
    };

    const run = async () => {
      await new Promise(r => setTimeout(r, 300));
      await SplashScreen.hideAsync();

      // Shimmer across K
      shimmer.value = withTiming(80, {
        duration: 2500,
        easing: Easing.inOut(Easing.ease),
      });

      // Hold centered K
      await new Promise(r => setTimeout(r, 600));

      // K slides left
      kTranslateX.value = withTiming(-K_SLIDE_X, {
        duration: 600,
        easing: Easing.inOut(Easing.cubic),
      });

      // RUSTY unfolds from K
      restOpacity.value = withDelay(200, withTiming(1, { duration: 300 }));
      restClipW.value = withDelay(200, withTiming(REST_TEXT_W, {
        duration: 700,
        easing: Easing.out(Easing.cubic),
      }));

      // Wait for unfold to finish
      await new Promise(r => setTimeout(r, 1100));

      // Fade out the overlay — the logo stays because KrustyLogo
      // in the app's empty state is in the same position underneath.
      // The UI components (top bar, chat bar) appear via entrance animations.
      overlayOpacity.value = withTiming(0, {
        duration: 400,
        easing: Easing.out(Easing.cubic),
      }, () => {
        runOnJS(finish)();
      });
    };

    run();
  }, []);

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

      {/* Splash overlay: BG + animated logo, fades out at end */}
      <Animated.View style={[styles.solidOverlay, fadeStyle]} pointerEvents="none">
        <View style={styles.center}>
          <Animated.View style={[styles.logoRow, kSlideStyle]}>
            {/* K portion */}
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

            {/* RUSTY — unfolds from K */}
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
});
