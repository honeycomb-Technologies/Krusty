import { useEffect, useState, useCallback, useRef } from 'react';
import { View, StyleSheet } from 'react-native';
import LottieView from 'lottie-react-native';
import * as SplashScreen from 'expo-splash-screen';

SplashScreen.preventAutoHideAsync();

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [done, setDone] = useState(false);
  const [ready, setReady] = useState(false);
  const lottieRef = useRef<LottieView>(null);

  const handleLayout = useCallback(() => {
    setReady(true);
  }, []);

  useEffect(() => {
    if (!ready) return;

    const timer = setTimeout(async () => {
      await SplashScreen.hideAsync();
      lottieRef.current?.play();
    }, 150);

    return () => clearTimeout(timer);
  }, [ready]);

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
  },
});
