import { useWindowDimensions } from 'react-native';

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

  const breakpoint: Breakpoint =
    width >= DESKTOP_MIN ? 'desktop' :
    width >= TABLET_MIN ? 'tablet' :
    'mobile';

  return {
    breakpoint,
    isMobile: breakpoint === 'mobile',
    isTablet: breakpoint === 'tablet',
    isDesktop: breakpoint !== 'mobile',
    width,
  };
}
