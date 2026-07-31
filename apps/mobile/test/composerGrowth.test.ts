import {
  COMPOSER_MAX_HEIGHT,
  COMPOSER_PILL_HEIGHT,
  INPUT_GROWTH_CHROME,
  INPUT_LINE_HEIGHT,
  estimateCompactInputHeight,
  resolveComposerBarHeight,
  shouldExpandComposerHeight,
} from '../components/chat/composerGrowth';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('single-line drafts stay at one visual row height', () => {
  const height = estimateCompactInputHeight('hello agent', 240);
  assert(height === INPUT_LINE_HEIGHT, `expected single-line height, got ${height}`);
  assert(!shouldExpandComposerHeight(height), 'single-line should stay compacted');
  assert(
    resolveComposerBarHeight(height, false) === COMPOSER_PILL_HEIGHT,
    'single-line bar should remain pill height',
  );
});

Deno.test('hard newlines grow the composer estimate and bar', () => {
  const height = estimateCompactInputHeight('line one\nline two\nline three', 240);
  assert(
    height === INPUT_LINE_HEIGHT * 3,
    `expected three visual rows, got ${height}`,
  );
  assert(shouldExpandComposerHeight(height), 'multi-line should expand');
  const barHeight = resolveComposerBarHeight(height, false);
  assert(barHeight > COMPOSER_PILL_HEIGHT, `expected expanded bar, got ${barHeight}`);
  assert(barHeight <= COMPOSER_MAX_HEIGHT, `bar exceeded max height: ${barHeight}`);
});

Deno.test('wrapped text grows beyond a single row', () => {
  const longLine =
    'this is a long draft that should wrap across multiple visual lines inside the compact composer';
  const height = estimateCompactInputHeight(longLine, 160);
  assert(height > INPUT_LINE_HEIGHT, `expected wrap growth, got ${height}`);
  assert(
    height <= COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME,
    `expected cap at max content height, got ${height}`,
  );
  assert(shouldExpandComposerHeight(height), 'wrapped text should expand');
});

Deno.test('empty drafts report zero content height and stay compacted', () => {
  assert(estimateCompactInputHeight('', 240) === 0, 'empty text must report 0');
  assert(
    resolveComposerBarHeight(0, false) === COMPOSER_PILL_HEIGHT,
    'empty drafts must keep the pill height',
  );
});

Deno.test('recording forces the pill height even with multi-line content', () => {
  const height = estimateCompactInputHeight('line one\nline two\nline three', 240);
  assert(
    resolveComposerBarHeight(height, true) === COMPOSER_PILL_HEIGHT,
    'recording should force the pill height',
  );
});
