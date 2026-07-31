import {
  COMPOSER_COLLAPSE_THRESHOLD,
  COMPOSER_EXPAND_THRESHOLD,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_PILL_HEIGHT,
  INPUT_EXPANDED_VERTICAL_PADDING,
  INPUT_GROWTH_CHROME,
  INPUT_LINE_HEIGHT,
  estimateCompactInputHeight,
  resolveComposerBarHeight,
  resolveComposerInputHeight,
  resolveNextInputContentHeight,
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
  assert(
    !shouldExpandComposerHeight(height, false),
    'single-line should stay compacted',
  );
  assert(
    resolveComposerBarHeight(height, false, COMPOSER_PILL_HEIGHT, false) ===
      COMPOSER_PILL_HEIGHT,
    'single-line bar should remain pill height',
  );
});

Deno.test('hard newlines grow the composer estimate and bar', () => {
  const height = estimateCompactInputHeight('line one\nline two\nline three', 240);
  assert(
    height === INPUT_LINE_HEIGHT * 3,
    `expected three visual rows, got ${height}`,
  );
  assert(shouldExpandComposerHeight(height, false), 'multi-line should expand');
  const inputHeight = resolveComposerInputHeight(height, true);
  const barHeight = resolveComposerBarHeight(
    height,
    false,
    COMPOSER_PILL_HEIGHT,
    true,
  );
  assert(inputHeight === height, `expected input height ${height}, got ${inputHeight}`);
  assert(barHeight > COMPOSER_PILL_HEIGHT, `expected expanded bar, got ${barHeight}`);
  assert(barHeight <= COMPOSER_MAX_HEIGHT, `bar exceeded max height: ${barHeight}`);
  assert(
    barHeight ===
      height + INPUT_EXPANDED_VERTICAL_PADDING * 2 + INPUT_GROWTH_CHROME,
    `expected text + padding + chrome, got ${barHeight}`,
  );
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
  assert(shouldExpandComposerHeight(height, false), 'wrapped text should expand');
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
    resolveComposerBarHeight(height, true, COMPOSER_PILL_HEIGHT, true) ===
      COMPOSER_PILL_HEIGHT,
    'recording should force the pill height',
  );
});

Deno.test('expand/collapse thresholds use hysteresis around wrap boundaries', () => {
  const expandAt = COMPOSER_EXPAND_THRESHOLD + 1;
  const stayExpandedAt = COMPOSER_COLLAPSE_THRESHOLD + 1;
  const collapseAt = COMPOSER_COLLAPSE_THRESHOLD;

  assert(
    shouldExpandComposerHeight(expandAt, false),
    'should expand once past expand threshold',
  );
  assert(
    shouldExpandComposerHeight(stayExpandedAt, true),
    'should remain expanded between collapse and expand thresholds',
  );
  assert(
    !shouldExpandComposerHeight(collapseAt, true),
    'should collapse only after falling through collapse threshold',
  );
  assert(
    !shouldExpandComposerHeight(COMPOSER_EXPAND_THRESHOLD, false),
    'exact expand threshold should not expand until strictly above',
  );
});

Deno.test('measured content height wins over later estimate samples', () => {
  const measured = resolveNextInputContentHeight({
    current: 44,
    next: 66,
    source: 'measured',
    hasMeasured: false,
  });
  assert(measured === 66, `expected measured growth, got ${measured}`);

  const blockedEstimate = resolveNextInputContentHeight({
    current: 66,
    next: 22,
    source: 'estimate',
    hasMeasured: true,
  });
  assert(
    blockedEstimate === 66,
    `estimate must not shrink after measurement, got ${blockedEstimate}`,
  );

  const noisyMeasured = resolveNextInputContentHeight({
    current: 66,
    next: 66.4,
    source: 'measured',
    hasMeasured: true,
  });
  assert(
    noisyMeasured === 66,
    `sub-pixel measured noise should be ignored, got ${noisyMeasured}`,
  );

  const bootstrapEstimate = resolveNextInputContentHeight({
    current: 0,
    next: 44,
    source: 'estimate',
    hasMeasured: false,
  });
  assert(
    bootstrapEstimate === 44,
    `bootstrap estimates should apply before measurement, got ${bootstrapEstimate}`,
  );
});
