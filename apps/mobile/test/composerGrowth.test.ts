import {
  COMPOSER_COLLAPSE_THRESHOLD,
  COMPOSER_EXPAND_THRESHOLD,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_PILL_HEIGHT,
  INPUT_EXPANDED_VERTICAL_PADDING,
  INPUT_GROWTH_CHROME,
  INPUT_LINE_HEIGHT,
  countVisualLinesForSegment,
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
  assert(
    resolveComposerInputHeight(height) === INPUT_LINE_HEIGHT,
    'single-line input height should be one row',
  );
});

Deno.test('hard newlines grow the composer estimate and bar', () => {
  const height = estimateCompactInputHeight('line one\nline two\nline three', 240);
  assert(
    height === INPUT_LINE_HEIGHT * 3,
    `expected three visual rows, got ${height}`,
  );
  assert(shouldExpandComposerHeight(height, false), 'multi-line should expand');
  const inputHeight = resolveComposerInputHeight(height);
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

Deno.test('soft-wrapped typing grows without hard newlines', () => {
  // No `\n` — this is the path that failed on device (paste worked, typing didn't).
  const longLine =
    'this is a long draft that should wrap across multiple visual lines inside the compact composer';
  const height = estimateCompactInputHeight(longLine, 160);
  assert(height > INPUT_LINE_HEIGHT, `expected wrap growth, got ${height}`);
  assert(
    height <=
      COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME - INPUT_EXPANDED_VERTICAL_PADDING * 2,
    `expected cap at max content height, got ${height}`,
  );
  assert(shouldExpandComposerHeight(height, false), 'wrapped text should expand');
  assert(
    resolveComposerInputHeight(height) === height,
    'input height must track soft-wrap content, not stay clamped to one line',
  );
  assert(
    resolveComposerBarHeight(height, false, COMPOSER_PILL_HEIGHT, true) >
      COMPOSER_PILL_HEIGHT,
    'bar must grow for soft wrap',
  );
});

Deno.test('soft-wrap expands by visual line 2 at typical composer widths', () => {
  // Regression: lag-until-line-4 on device. Word-aware estimate must open the
  // bar once the second visual row is needed, not after four lines of typing.
  const widths = [160, 200, 240, 280];
  for (const width of widths) {
    // Build text that is just over one visual row of average words.
    let text = 'typing';
    let height = estimateCompactInputHeight(text, width);
    let guard = 0;
    while (height <= INPUT_LINE_HEIGHT && guard < 80) {
      text += ' word';
      height = estimateCompactInputHeight(text, width);
      guard += 1;
    }
    assert(
      shouldExpandComposerHeight(height, false),
      `width=${width}: should expand once second visual row is needed (text len=${text.length}, h=${height})`,
    );
    // Must not require absurd length (~4+ pure character rows at optimistic width).
    assert(
      text.length < 120,
      `width=${width}: expanded too late (len=${text.length})`,
    );
  }
});

Deno.test('word-aware wrap breaks earlier than pure character packing for long words', () => {
  // A long token near the line capacity should force mid-token wraps.
  const lines = countVisualLinesForSegment(
    'hello supercalifragilisticexpialidocious world',
    12,
  );
  assert(lines >= 3, `expected multi-line wrap for long token, got ${lines}`);
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

Deno.test('estimate may grow after measurement but must not shrink', () => {
  // Soft wrap: first measured sample is often still ~one line while the view
  // is clamped. Estimate must still be allowed to open the field.
  const grown = resolveNextInputContentHeight({
    current: 22,
    next: 44,
    source: 'estimate',
    hasMeasured: true,
  });
  assert(grown === 44, `estimate must grow after measurement, got ${grown}`);

  const blockedShrink = resolveNextInputContentHeight({
    current: 66,
    next: 22,
    source: 'estimate',
    hasMeasured: true,
  });
  assert(
    blockedShrink === 66,
    `estimate must not shrink after measurement, got ${blockedShrink}`,
  );

  const bootstrapShrink = resolveNextInputContentHeight({
    current: 44,
    next: 22,
    source: 'estimate',
    hasMeasured: false,
  });
  assert(
    bootstrapShrink === 22,
    `estimate may shrink before measurement, got ${bootstrapShrink}`,
  );

  const measured = resolveNextInputContentHeight({
    current: 44,
    next: 66,
    source: 'measured',
    hasMeasured: false,
  });
  assert(measured === 66, `expected measured growth, got ${measured}`);

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
});

Deno.test('input height always tracks content so soft wrap can remeasure', () => {
  // Regression: clamping input to one line while collapsed made contentSize
  // stick and hid typed wrap. Input height must follow content always.
  assert(
    resolveComposerInputHeight(INPUT_LINE_HEIGHT * 2) === INPUT_LINE_HEIGHT * 2,
    'two-line content must size the input to two lines even if bar not expanded yet',
  );
});
