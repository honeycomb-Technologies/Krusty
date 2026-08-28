import { useEffect, useState, type ComponentType } from 'react';

import { FAB_GOOEY_ENABLED, type GooeyProgresses } from './fabGooey';

type LayerProps = {
  progresses: GooeyProgresses;
  pillCount: number;
  fill: string;
};

/**
 * Defer the Skia surface one frame so a FAB tap cannot create it
 * on the same JS turn as the accordion open. Core stays behind the flag.
 */
export function FabGooeyLayer(props: LayerProps) {
  const [Impl, setImpl] = useState<ComponentType<LayerProps> | null>(null);

  useEffect(() => {
    if (!FAB_GOOEY_ENABLED) return;
    let cancelled = false;
    const frame = requestAnimationFrame(() => {
      void import('./FabGooeyLayerCore')
        .then((mod) => {
          if (!cancelled) setImpl(() => mod.FabGooeyLayer);
        })
        .catch(() => {
          // Accordion icons still work without the silhouette.
        });
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(frame);
    };
  }, []);

  if (!FAB_GOOEY_ENABLED || !Impl) return null;
  return <Impl {...props} />;
}
