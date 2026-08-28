import { WithSkiaWeb } from '@shopify/react-native-skia/lib/module/web';

import { FAB_GOOEY_ENABLED, type GooeyProgresses } from './fabGooey';

const loadGooeyLayer = async () => {
  const module = await import('./FabGooeyLayerCore');
  return { default: module.FabGooeyLayer };
};

export function FabGooeyLayer(props: {
  progresses: GooeyProgresses;
  pillCount: number;
  fill: string;
}) {
  if (!FAB_GOOEY_ENABLED) return null;
  return (
    <WithSkiaWeb
      getComponent={loadGooeyLayer}
      componentProps={props}
      fallback={null}
    />
  );
}
