import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';

interface SplashContextValue {
  splashDone: boolean;
  markSplashDone: () => void;
}

const SplashContext = createContext<SplashContextValue>({
  splashDone: false,
  markSplashDone: () => {},
});

export function SplashProvider({ children }: { children: ReactNode }) {
  const [splashDone, setSplashDone] = useState(false);
  const markSplashDone = useCallback(() => setSplashDone(true), []);

  return (
    <SplashContext.Provider value={{ splashDone, markSplashDone }}>
      {children}
    </SplashContext.Provider>
  );
}

export function useSplashState() {
  return useContext(SplashContext);
}
