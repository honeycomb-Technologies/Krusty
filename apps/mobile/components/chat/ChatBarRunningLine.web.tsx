import { lazy, memo, Suspense } from 'react';
import type { StyleProp, ViewStyle } from 'react-native';

export const RUN_LINE_CORNER_CLIMB = 35;

export interface ChatBarRunningLineProps {
  active: boolean;
  width: number;
  cornerClimb: number;
  theme?: 'dark' | 'light';
  style?: StyleProp<ViewStyle>;
}

const ActiveSkiaRunningLine = lazy(
  () => import('./ChatBarRunningLineSkia'),
);

function ChatBarRunningLineWeb(props: ChatBarRunningLineProps) {
  if (!props.active) return null;

  return (
    <Suspense fallback={null}>
      <ActiveSkiaRunningLine {...props} />
    </Suspense>
  );
}

export const ChatBarRunningLine = memo(ChatBarRunningLineWeb);
