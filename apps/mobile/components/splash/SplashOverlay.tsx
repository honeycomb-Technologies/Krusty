import { useEffect, useState, useCallback } from 'react';
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

  useEffect(() => {
    const timer = setTimeout(() => {
      SplashScreen.hideAsync();
    }, 100);
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
        source={require('../../assets/animations/splash.json')}
        autoPlay
        loop={false}
        onAnimationFinish={handleFinish}
        style={styles.overlay}
        resizeMode="cover"
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
