import { WithSkiaWeb } from '@shopify/react-native-skia/lib/module/web';

import type { ChatBarRunningLineProps } from './ChatBarRunningLine.web';

const loadSkiaRunningLine = async () => {
  const module = await import('./ChatBarRunningLine.native');
  return { default: module.ChatBarRunningLine };
};

export default function ChatBarRunningLineSkiaWeb(
  props: ChatBarRunningLineProps,
) {
  return (
    <WithSkiaWeb
      getComponent={loadSkiaRunningLine}
      componentProps={props}
      fallback={null}
    />
  );
}
