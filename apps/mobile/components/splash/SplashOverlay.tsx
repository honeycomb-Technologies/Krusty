import {
  useEffect,
  useState,
  useCallback,
  useRef,
  type ComponentRef,
} from 'react';
import { View, StyleSheet } from 'react-native';
import LottieView from 'lottie-react-native';
import * as SplashScreen from 'expo-splash-screen';

SplashScreen.preventAutoHideAsync();

const DELAY_BEFORE_PLAY_MS = 400;

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);
  const lottieRef = useRef<ComponentRef<typeof LottieView>>(null);

  const handleLayout = useCallback(() => {
    SplashScreen.hideAsync();

    const timer = setTimeout(() => {
      lottieRef.current?.play();
    }, DELAY_BEFORE_PLAY_MS);

    return () => clearTimeout(timer);
  }, []);

  const handleFinish = useCallback(
    (isCancelled: boolean) => {
      if (!isCancelled) {
        setDone(true);
        onComplete?.();
      }
    },
    [onComplete],
  );

  if (done) return <>{children}</>;

  return (
    <View style={styles.root}>
      <View style={StyleSheet.absoluteFill}>{children}</View>
      <LottieView
        ref={lottieRef}
        source={require('../../assets/animations/splash.json')}
        loop={false}
        onAnimationFinish={handleFinish}
        onLayout={handleLayout}
        style={styles.overlay}
        resizeMode="cover"
        progress={0}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  overlay: {
    ...StyleSheet.absoluteFillObject,
    zIndex: 10,
    backgroundColor: '#0b1119',
  },
});
