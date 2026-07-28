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
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('Krusty line palette keeps the product accent as its lead color', () => {
  const lead = parseCssColor(lineSpec.palettes.line.sunset.dark[0].color);
  assert(Math.round(lead.r * 255) === 255, 'lead red channel should remain 255');
  assert(Math.round(lead.g * 255) === 107, 'lead green channel should remain 107');
  assert(Math.round(lead.b * 255) === 53, 'lead blue channel should remain 53');
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
