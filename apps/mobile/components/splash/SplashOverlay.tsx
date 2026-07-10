import {
  useEffect,
  useState,
  useCallback,
  useRef,
  type ComponentRef,
  type CSSProperties,
} from 'react';
import { Animated, Platform, View, StyleSheet } from 'react-native';
import LottieView from 'lottie-react-native';
import * as SplashScreen from 'expo-splash-screen';

SplashScreen.preventAutoHideAsync();

const DELAY_BEFORE_PLAY_MS = 400;
const FALLBACK_COMPLETE_MS = 5000;
const EXIT_FADE_MS = 260;
const SPLASH_BACKGROUND = '#0b1119';

const webOverlayStyle: CSSProperties = {
  position: 'absolute',
  inset: 0,
  width: '100%',
  height: '100%',
  backgroundColor: SPLASH_BACKGROUND,
  zIndex: 10,
};

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [overlayVisible, setOverlayVisible] = useState(true);
  const lottieRef = useRef<ComponentRef<typeof LottieView>>(null);
  const completedRef = useRef(false);
  const playTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const overlayOpacityRef = useRef(new Animated.Value(1));

  const completeSplash = useCallback(() => {
    if (completedRef.current) return;
    completedRef.current = true;
    onComplete?.();
    Animated.timing(overlayOpacityRef.current, {
      toValue: 0,
      duration: EXIT_FADE_MS,
      useNativeDriver: false,
    }).start(() => setOverlayVisible(false));
  }, [onComplete]);

  useEffect(() => {
    const fallbackTimer = setTimeout(completeSplash, FALLBACK_COMPLETE_MS);
    return () => {
      clearTimeout(fallbackTimer);
      if (playTimerRef.current) {
        clearTimeout(playTimerRef.current);
      }
      overlayOpacityRef.current.stopAnimation();
    };
  }, [completeSplash]);

  const handleLayout = useCallback(() => {
    SplashScreen.hideAsync();

    if (Platform.OS === 'web') {
      return;
    }

    if (playTimerRef.current) {
      clearTimeout(playTimerRef.current);
    }
    playTimerRef.current = setTimeout(() => {
      lottieRef.current?.play();
    }, DELAY_BEFORE_PLAY_MS);
  }, []);

  const handleFinish = useCallback(
    (isCancelled: boolean) => {
      if (!isCancelled) {
        completeSplash();
      }
    },
    [completeSplash],
  );

  const handleLoadFailure = useCallback(
    (error: string) => {
      console.warn(`Splash animation failed to load: ${error}`);
      completeSplash();
    },
    [completeSplash],
  );

  useEffect(() => {
    if (Platform.OS !== 'web') return;
    SplashScreen.hideAsync();
  }, []);

  return (
    <View style={styles.root}>
      <View style={StyleSheet.absoluteFill}>{children}</View>
      {overlayVisible ? (
        <Animated.View
          pointerEvents="none"
          style={[styles.overlayLayer, { opacity: overlayOpacityRef.current }]}
        >
          {Platform.OS === 'web' ? (
            <LottieView
              source={require('../../assets/animations/splash.json')}
              loop={false}
              autoPlay
              onAnimationFinish={handleFinish}
              onAnimationFailure={handleLoadFailure}
              webStyle={webOverlayStyle}
            />
          ) : (
            <LottieView
              ref={lottieRef}
              source={require('../../assets/animations/splash.json')}
              loop={false}
              onAnimationFinish={handleFinish}
              onAnimationFailure={handleLoadFailure}
              onLayout={handleLayout}
              style={styles.overlay}
              resizeMode="cover"
              progress={0}
            />
          )}
        </Animated.View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: SPLASH_BACKGROUND,
  },
  overlayLayer: {
    ...StyleSheet.absoluteFillObject,
    zIndex: 10,
    backgroundColor: SPLASH_BACKGROUND,
  },
  overlay: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: SPLASH_BACKGROUND,
  },
});
