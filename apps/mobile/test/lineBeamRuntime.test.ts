import lineSpec from '../components/chat/border-beam/line-spec.json' with {
  type: 'json',
};
import {
  BLOB_FLOATS,
  lineFrameValues,
  parseCssColor,
  simpleBlob,
  type LineKeyframeTables,
} from '../components/chat/border-beam/line-runtime';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('Mitsuro line palette stays within the violet spectrum', () => {
  for (const theme of ['dark', 'light'] as const) {
    for (const entry of lineSpec.palettes.line.violet[theme]) {
      const color = parseCssColor(entry.color);
      assert(
        color.b > color.g && color.r > color.g,
        `${theme} beam color ${entry.color} should remain violet`,
      );
    }

    for (const gradient of lineSpec.line.bloomGradients.violet[theme]) {
      for (const stop of gradient.stops) {
        if (stop.r === stop.g && stop.g === stop.b) continue;
        assert(
          stop.b > stop.g && stop.r > stop.g,
          `${theme} bloom color rgb(${stop.r}, ${stop.g}, ${stop.b}) should remain violet`,
        );
      }
    }
  }

  const highlight = lineSpec.line.whiteHighlight.dark.color;
  assert(
    highlight[2] > highlight[1] && highlight[0] > highlight[1],
    'dark beam highlight should be explicitly violet rather than white',
  );
});

Deno.test('web beam lazily loads the native Skia renderer', async () => {
  const wrapperSource = await Deno.readTextFile(
    new URL('../components/chat/ChatBarRunningLine.web.tsx', import.meta.url),
  );
  const skiaSource = await Deno.readTextFile(
    new URL('../components/chat/ChatBarRunningLineSkia.web.tsx', import.meta.url),
  );

  assert(
    wrapperSource.includes("lazy(")
      && wrapperSource.includes("import('./ChatBarRunningLineSkia')"),
    'idle web sessions should code-split the entire Skia loader',
  );
  assert(
    skiaSource.includes('WithSkiaWeb')
      && skiaSource.includes("import('./ChatBarRunningLine.native')"),
    'web beam should reuse the production Skia renderer',
  );
  assert(
    wrapperSource.includes('if (!props.active) return null'),
    'idle sessions must not download or mount CanvasKit',
  );
});

Deno.test('line beam is fully visible and centered at the midpoint', () => {
  const frame = lineFrameValues(
    lineSpec.line.keyframes as LineKeyframeTables,
    lineSpec.defaults.duration.line / 2,
    lineSpec.defaults.duration.line,
  );
  assert(Math.abs(frame.x - 0.5) < 0.001, 'mid-cycle beam should be centered');
  assert(frame.edge === 1, 'mid-cycle beam should be fully visible');
  assert(frame.w === 1.5, 'mid-cycle beam should reach peak width');
});

Deno.test('shader blob records keep the fixed uniform stride', () => {
  const blob = simpleBlob(10, 12, 20, 22, 1, 0.5, 0.25, 0.8);
  assert(blob.length === BLOB_FLOATS, 'blob record must match shader uniform stride');
  assert(blob[0] === 10 && blob[3] === 22, 'blob geometry should be preserved');
});

Deno.test('EAS Bun installs trust the Skia binary lifecycle', async () => {
  const packageJson = JSON.parse(
    await Deno.readTextFile(new URL('../package.json', import.meta.url)),
  ) as { trustedDependencies?: string[] };
  const bunLock = await Deno.readTextFile(
    new URL('../bun.lock', import.meta.url),
  );

  assert(
    packageJson.trustedDependencies?.includes('@shopify/react-native-skia'),
    'package.json must allow Skia to download its native binaries',
  );
  assert(
    bunLock.includes('"trustedDependencies"')
      && bunLock.includes('"@shopify/react-native-skia"'),
    'the frozen Bun lockfile must preserve Skia lifecycle trust for EAS',
  );
});
