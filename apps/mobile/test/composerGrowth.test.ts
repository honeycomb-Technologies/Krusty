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
  normalizeMeasuredContentHeight,
  resolveComposerBarHeight,
  resolveComposerContentHeight,
  resolveComposerInputHeight,
  resolveMeasuredInputContentHeight,
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
  const contentBox = resolveComposerContentHeight(height);
  const inputHeight = resolveComposerInputHeight(height, true);
  const barHeight = resolveComposerBarHeight(
    height,
    false,
    COMPOSER_PILL_HEIGHT,
    true,
  );
  assert(contentBox === height, `expected content box ${height}, got ${contentBox}`);
  assert(
    inputHeight === height + INPUT_EXPANDED_VERTICAL_PADDING * 2,
    `expected input height to include padding, got ${inputHeight}`,
  );
  assert(barHeight > COMPOSER_PILL_HEIGHT, `expected expanded bar, got ${barHeight}`);
  assert(barHeight <= COMPOSER_MAX_HEIGHT, `bar exceeded max height: ${barHeight}`);
  assert(
    barHeight === inputHeight + INPUT_GROWTH_CHROME,
    `expected input view + chrome, got ${barHeight}`,
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
    resolveComposerContentHeight(height) === height,
    'content box must track soft-wrap content, not stay clamped to one line',
  );
  assert(
    resolveComposerInputHeight(height, true) ===
      height + INPUT_EXPANDED_VERTICAL_PADDING * 2,
    'expanded input height must include padding inside RN height box',
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

Deno.test('estimate opens soft wrap but does not pin a tall floor', () => {
  // Soft wrap: first measured sample is often still ~one line while the view
  // is clamped. Estimate must still be allowed to open the field.
  const grown = resolveNextInputContentHeight({
    current: 22,
    next: 44,
    source: 'estimate',
    hasMeasured: true,
  });
  assert(grown === 44, `estimate must grow after measurement, got ${grown}`);

  // Once multi-line measured content is authoritative, estimates must not
  // shrink the bar — measured samples own shrink (or empty-text reset).
  const blockedShrink = resolveNextInputContentHeight({
    current: 66,
    next: 22,
    source: 'estimate',
    hasMeasured: true,
    measuredAuthoritative: true,
  });
  assert(
    blockedShrink === 66,
    `estimate must not shrink once measured is authoritative, got ${blockedShrink}`,
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

  // Still single-line bootstrap: estimate may follow the draft down.
  const preAuthShrink = resolveNextInputContentHeight({
    current: 44,
    next: 22,
    source: 'estimate',
    hasMeasured: true,
    measuredAuthoritative: false,
  });
  assert(
    preAuthShrink === 22,
    `estimate may shrink before multi-line measure, got ${preAuthShrink}`,
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
  // stick and hid typed wrap. Content box must follow content always.
  assert(
    resolveComposerContentHeight(INPUT_LINE_HEIGHT * 2) === INPUT_LINE_HEIGHT * 2,
    'two-line content must size the content box to two lines even if bar not expanded yet',
  );
  // RN includes paddingVertical inside explicit height, so the first expanded
  // multi-line heights must add pad or only ~28px of a 44px field is usable.
  assert(
    resolveComposerInputHeight(INPUT_LINE_HEIGHT * 2, true) ===
      INPUT_LINE_HEIGHT * 2 + INPUT_EXPANDED_VERTICAL_PADDING * 2,
    'expanded two-line input must reserve padding inside the height box',
  );
  assert(
    resolveComposerInputHeight(INPUT_LINE_HEIGHT, false) === INPUT_LINE_HEIGHT,
    'collapsed single-line input stays one row without expanded padding',
  );
});

Deno.test('padding-inclusive contentSize normalizes to pure line-stack height', () => {
  const pad = INPUT_EXPANDED_VERTICAL_PADDING * 2;
  const twoLines = INPUT_LINE_HEIGHT * 2;
  const threeLines = INPUT_LINE_HEIGHT * 3;

  // Expanded field: native often reports text + vertical padding.
  const normalizedTwo = normalizeMeasuredContentHeight(twoLines + pad, {
    currentlyExpanded: true,
    estimatedHeight: twoLines,
  });
  assert(
    normalizedTwo === twoLines,
    `expected pad stripped to ${twoLines}, got ${normalizedTwo}`,
  );

  const normalizedThree = normalizeMeasuredContentHeight(threeLines + pad, {
    currentlyExpanded: true,
    estimatedHeight: threeLines,
  });
  assert(
    normalizedThree === threeLines,
    `expected pad stripped to ${threeLines}, got ${normalizedThree}`,
  );

  // Pure text samples must pass through unchanged.
  assert(
    normalizeMeasuredContentHeight(twoLines, {
      currentlyExpanded: true,
      estimatedHeight: twoLines,
    }) === twoLines,
    'pure two-line measure must not strip',
  );
});

Deno.test('measured path stabilizes 2→3 line padding thrash', () => {
  const pad = INPUT_EXPANDED_VERTICAL_PADDING * 2;
  const twoLines = INPUT_LINE_HEIGHT * 2;
  const threeLines = INPUT_LINE_HEIGHT * 3;

  // Simulate the thrash loop: pure ↔ padded contentSize at two lines, then
  // advance to three. Heights must stay line-aligned and bar steps 68 → 90.
  let content = resolveMeasuredInputContentHeight({
    current: INPUT_LINE_HEIGHT,
    measuredHeight: twoLines + pad,
    estimatedHeight: twoLines,
    currentlyExpanded: false,
  });
  assert(content === twoLines, `two-line open should settle at ${twoLines}, got ${content}`);

  // Oscillating pad-inflated samples must not inflate stored content.
  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: twoLines,
    estimatedHeight: twoLines,
    currentlyExpanded: true,
  });
  assert(content === twoLines, `pure remeasure must stay at ${twoLines}, got ${content}`);

  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: twoLines + pad,
    estimatedHeight: twoLines,
    currentlyExpanded: true,
  });
  assert(
    content === twoLines,
    `padded remeasure must not re-inflate past ${twoLines}, got ${content}`,
  );

  // Sub-half-line measured shrink noise is ignored.
  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: twoLines - 8,
    estimatedHeight: twoLines,
    currentlyExpanded: true,
  });
  assert(
    content === twoLines,
    `sub-line measured shrink must be ignored, got ${content}`,
  );

  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: threeLines + pad,
    estimatedHeight: threeLines,
    currentlyExpanded: true,
  });
  assert(content === threeLines, `three-line step should settle at ${threeLines}, got ${content}`);

  const barTwo = resolveComposerBarHeight(twoLines, false, COMPOSER_PILL_HEIGHT, true);
  const barThree = resolveComposerBarHeight(threeLines, false, COMPOSER_PILL_HEIGHT, true);
  assert(
    barTwo === twoLines + pad + INPUT_GROWTH_CHROME,
    `expected stable two-line bar 68, got ${barTwo}`,
  );
  assert(
    barThree === threeLines + pad + INPUT_GROWTH_CHROME,
    `expected stable three-line bar 90, got ${barThree}`,
  );
  assert(barThree - barTwo === INPUT_LINE_HEIGHT, `2→3 bar step must be one line, got ${barThree - barTwo}`);
});

Deno.test('measured shrink of a full line still collapses content', () => {
  // Real line deletion (22px) must still shrink; only sub-half-line noise is sticky.
  const shrunk = resolveNextInputContentHeight({
    current: INPUT_LINE_HEIGHT * 3,
    next: INPUT_LINE_HEIGHT * 2,
    source: 'measured',
    hasMeasured: true,
  });
  assert(
    shrunk === INPUT_LINE_HEIGHT * 2,
    `full-line measured shrink must apply, got ${shrunk}`,
  );

  const sticky = resolveNextInputContentHeight({
    current: INPUT_LINE_HEIGHT * 2,
    next: INPUT_LINE_HEIGHT * 2 - 8,
    source: 'measured',
    hasMeasured: true,
  });
  assert(
    sticky === INPUT_LINE_HEIGHT * 2,
    `8px measured shrink must stick at current, got ${sticky}`,
  );
});

Deno.test('measured path shrinks with draft instead of estimate floor', () => {
  const pad = INPUT_EXPANDED_VERTICAL_PADDING * 2;
  const oneLine = INPUT_LINE_HEIGHT;
  const twoLines = INPUT_LINE_HEIGHT * 2;
  const threeLines = INPUT_LINE_HEIGHT * 3;
  const fourLines = INPUT_LINE_HEIGHT * 4;

  // Regression: flooring measured at estimate kept the bar at max until the
  // draft was fully cleared. Measured multi-line samples must win on shrink.
  let content = resolveMeasuredInputContentHeight({
    current: oneLine,
    measuredHeight: fourLines + pad,
    estimatedHeight: fourLines,
    currentlyExpanded: true,
  });
  assert(content === fourLines, `open at four lines, got ${content}`);

  // Estimate still claims four lines (stale aggressive wrap), measured says three.
  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: threeLines + pad,
    estimatedHeight: fourLines,
    currentlyExpanded: true,
  });
  assert(
    content === threeLines,
    `measured three-line shrink must beat stale estimate floor, got ${content}`,
  );

  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: twoLines,
    estimatedHeight: threeLines,
    currentlyExpanded: true,
  });
  assert(content === twoLines, `measured two-line shrink, got ${content}`);

  content = resolveMeasuredInputContentHeight({
    current: content,
    measuredHeight: oneLine,
    estimatedHeight: oneLine,
    currentlyExpanded: true,
  });
  assert(content === oneLine, `measured one-line shrink, got ${content}`);

  // Soft-wrap stall: measured stuck at one line while estimate needs two.
  const opened = resolveMeasuredInputContentHeight({
    current: oneLine,
    measuredHeight: oneLine,
    estimatedHeight: twoLines,
    currentlyExpanded: false,
  });
  assert(
    opened === twoLines,
    `clamped one-line measure must open via estimate bootstrap, got ${opened}`,
  );
});
