import { Platform, useWindowDimensions } from 'react-native';

export type Breakpoint = 'mobile' | 'tablet' | 'desktop';

const TABLET_MIN = 768;
const DESKTOP_MIN = 1024;

export function useBreakpoint(): {
  breakpoint: Breakpoint;
  isMobile: boolean;
  isTablet: boolean;
  isDesktop: boolean;
  width: number;
} {
  const { width } = useWindowDimensions();

  // Native iOS/Android windows remain touch-first even when a tablet,
  // Waydroid, DeX, or Stage Manager reports a desktop-sized viewport. The
  // desktop shell is the web/Tauri surface, where hover and split panes exist.
  const breakpoint: Breakpoint =
    Platform.OS === 'web' && width >= DESKTOP_MIN ? 'desktop' :
    width >= TABLET_MIN ? 'tablet' :
    'mobile';

  return {
    breakpoint,
    isMobile: breakpoint === 'mobile',
    isTablet: breakpoint === 'tablet',
    isDesktop: breakpoint === 'desktop',
    width,
  };
}
