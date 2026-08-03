/** Composer pill height (matches ChatBar `PILL`). */
export const COMPOSER_PILL_HEIGHT = 56;
export const COMPOSER_MAX_HEIGHT = 112;
export const INPUT_GROWTH_CHROME = 8;
export const INPUT_LINE_HEIGHT = 22;
export const INPUT_EXPANDED_VERTICAL_PADDING = 8;
/**
 * Approximate average glyph width for 16px system UI fonts on iOS.
 * Intentionally slightly aggressive (narrower than true average) so soft-wrap
 * opens the field by visual line 2 rather than lagging until ~line 4.
 * Soft-wrap growth relies on this until native contentSize reports a
 * multi-line height (iOS often keeps contentSize stuck at the view height
 * while the field is still clamped to one line).
 */
export const COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH = 8.2;
/**
 * Extra conservatism for word-aware wrapping: real UIFont word-wrap breaks
 * earlier than pure character packing when tokens are long.
 */
export const COMPACT_INPUT_WORD_WRAP_FACTOR = 0.92;
/** Expand once content clearly needs a second visual row. */
export const COMPOSER_EXPAND_THRESHOLD = INPUT_LINE_HEIGHT + 4;
/**
 * Collapse only after content is back near a single centered line.
 * The gap vs expand threshold prevents thrashing around wrap boundaries.
 */
export const COMPOSER_COLLAPSE_THRESHOLD = INPUT_LINE_HEIGHT + 1;
/** Ignore tiny content-size jitter from iOS UITextView measurement noise. */
export const COMPOSER_HEIGHT_EPSILON = 1;
/**
 * Ignore measured shrinks smaller than half a line. Padding/view remeasures
 * often bounce 8–16px without a real line change; full line steps stay intact.
 */
export const COMPOSER_MEASURED_SHRINK_MIN = INPUT_LINE_HEIGHT / 2;

/** Max content-box height (line stack only, no expanded padding / bar chrome). */
export function maxComposerContentHeight(): number {
  return (
    COMPOSER_MAX_HEIGHT -
    INPUT_GROWTH_CHROME -
    INPUT_EXPANDED_VERTICAL_PADDING * 2
  );
}

/**
 * Snap a content-box height to whole visual lines.
 * Keeps bar steps on a stable 22px grid instead of fractional thrash.
 */
export function snapContentHeightToLines(contentHeight: number): number {
  if (contentHeight <= 0) return 0;
  const maxContent = maxComposerContentHeight();
  const lines = Math.max(
    1,
    Math.round(contentHeight / INPUT_LINE_HEIGHT),
  );
  return Math.min(maxContent, lines * INPUT_LINE_HEIGHT);
}

/**
 * Normalize a native `contentSize.height` sample to content-box height.
 *
 * The expanded TextInput applies vertical padding *inside* its explicit height.
 * On iOS/Android, `onContentSizeChange` often reports text + that padding (or
 * tracks the view box). Feeding that straight back into height math double-counts
 * pad and produces oversized empty space under the draft.
 *
 * Returns line-stack height only — never includes expanded vertical padding.
 */
export function normalizeMeasuredContentHeight(
  measuredHeight: number,
  options: {
    currentlyExpanded?: boolean;
    estimatedHeight?: number;
  } = {},
): number {
  if (measuredHeight <= 0) return 0;

  const maxContent = maxComposerContentHeight();
  const pad = INPUT_EXPANDED_VERTICAL_PADDING * 2;
  const estimated = Math.max(0, options.estimatedHeight ?? 0);
  const raw = Math.ceil(measuredHeight);

  const pure = Math.min(maxContent, raw);
  const stripped = Math.min(maxContent, Math.max(0, raw - pad));

  const nearestLines = (value: number) =>
    Math.max(
      INPUT_LINE_HEIGHT,
      Math.round(value / INPUT_LINE_HEIGHT) * INPUT_LINE_HEIGHT,
    );

  const strippedLines = nearestLines(Math.max(INPUT_LINE_HEIGHT, stripped));
  const distance = (a: number, b: number) => Math.abs(a - b);

  // Raw sample sits on (N lines + pad) while stripped sits on N lines.
  const rawLooksLikePaddedLines =
    stripped >= INPUT_LINE_HEIGHT - COMPOSER_HEIGHT_EPSILON &&
    distance(stripped, strippedLines) <= 3 &&
    distance(raw, strippedLines + pad) <= 3;

  // Prefer the candidate closer to the soft-wrap estimate.
  const strippedCloserToEstimate =
    estimated > 0 &&
    distance(stripped, estimated) + COMPOSER_HEIGHT_EPSILON <
      distance(pure, estimated);

  // View-capped / oversize samples that only fit after stripping pad.
  const pureOvershootsMax =
    raw > maxContent && stripped <= maxContent + COMPOSER_HEIGHT_EPSILON;

  // Expanded field + estimate already multi-line: pad-inflated pure sample.
  const expandedPaddedOvershoot =
    options.currentlyExpanded === true &&
    estimated > COMPOSER_EXPAND_THRESHOLD &&
    pure > estimated + pad / 2 &&
    distance(stripped, estimated) <= INPUT_LINE_HEIGHT / 2;

  // Expanded field reporting near the current view height rather than text.
  // contentSize that lands on contentBox+pad while estimate is shorter is view tracking.
  const looksLikeViewHeight =
    options.currentlyExpanded === true &&
    estimated > 0 &&
    pure >= estimated + pad - COMPOSER_HEIGHT_EPSILON &&
    pure <= estimated + pad + INPUT_LINE_HEIGHT / 2 + COMPOSER_HEIGHT_EPSILON;

  const shouldStrip =
    rawLooksLikePaddedLines ||
    strippedCloserToEstimate ||
    pureOvershootsMax ||
    expandedPaddedOvershoot ||
    looksLikeViewHeight;

  const contentBox = shouldStrip ? stripped : pure;
  return snapContentHeightToLines(
    Math.min(maxContent, Math.max(0, contentBox)),
  );
}

/**
 * Whether a measured sample still looks single-line-clamped while the draft
 * clearly needs more rows (classic iOS soft-wrap stall).
 */
export function measuredLooksSoftWrapClamped(
  normalizedMeasured: number,
  estimatedHeight: number,
): boolean {
  return (
    normalizedMeasured > 0 &&
    normalizedMeasured <= INPUT_LINE_HEIGHT + COMPOSER_HEIGHT_EPSILON &&
    estimatedHeight > COMPOSER_EXPAND_THRESHOLD
  );
}

/**
 * Full measured-sample pipeline.
 *
 * Standard auto-grow model (iOS/RN):
 * - Width is fixed by layout; text wraps inside that width.
 * - Height follows contentSize (content-box), min one line, max composer cap.
 *
 * Estimate is only a bootstrap when iOS keeps contentSize stuck at one line
 * while soft wrap already needs more. Once measured is multi-line (or not
 * clamped), measured is authoritative for both grow and shrink.
 *
 * Do NOT floor measured at estimate forever — that ratcheted the bar tall and
 * refused to shrink until the draft was fully cleared.
 */
export function resolveMeasuredInputContentHeight(options: {
  current: number;
  measuredHeight: number;
  estimatedHeight: number;
  currentlyExpanded: boolean;
}): number {
  const estimated = Math.max(0, options.estimatedHeight);
  const normalized = normalizeMeasuredContentHeight(options.measuredHeight, {
    currentlyExpanded: options.currentlyExpanded,
    estimatedHeight: estimated,
  });

  const next = measuredLooksSoftWrapClamped(normalized, estimated)
    ? snapContentHeightToLines(estimated)
    : normalized;

  return resolveNextInputContentHeight({
    current: options.current,
    next,
    source: 'measured',
    hasMeasured: true,
  });
}

/**
 * Count visual lines for one hard-break segment using word-aware packing.
 * Long unbroken tokens (URLs, pasted identifiers) still force mid-token wraps.
 */
export function countVisualLinesForSegment(
  segment: string,
  charactersPerLine: number,
): number {
  const cpl = Math.max(8, charactersPerLine);
  if (!segment) return 1;

  const words = segment.split(/(\s+)/);
  let lines = 1;
  let col = 0;

  for (const word of words) {
    if (!word) continue;
    // Whitespace at end of line can wrap without adding a visible column.
    if (/^\s+$/.test(word)) {
      if (col + word.length > cpl && col > 0) {
        lines += 1;
        col = 0;
      } else {
        col += word.length;
      }
      continue;
    }

    // Oversized token: wrap mid-word like UITextView.
    if (word.length > cpl) {
      if (col > 0) {
        lines += 1;
        col = 0;
      }
      const full = Math.floor(word.length / cpl);
      lines += full;
      col = word.length % cpl;
      if (col === 0 && full > 0) {
        // Ended exactly on a boundary; next glyph starts a new line.
        col = 0;
      }
      continue;
    }

    if (col > 0 && col + word.length > cpl) {
      lines += 1;
      col = word.length;
    } else {
      col += word.length;
    }
  }

  return Math.max(1, lines);
}

/**
 * Approximate the text block height for composer growth.
 * Critical for soft wrap: typing does not insert `\n`, so this character
 * wrap estimate is what opens the field when contentSize is still capped.
 */
export function estimateCompactInputHeight(
  value: string,
  inputWidth: number,
): number {
  if (!value) return 0;
  const usableWidth = Math.max(48, inputWidth);
  const rawCpl = Math.floor(usableWidth / COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH);
  // Word-wrap factor makes the effective line capacity slightly smaller so
  // soft wrap expands earlier on device (fix for lag-until-line-4).
  const charactersPerLine = Math.max(
    8,
    Math.floor(rawCpl * COMPACT_INPUT_WORD_WRAP_FACTOR),
  );
  const visualLineCount = value.split('\n').reduce(
    (total, line) => total + countVisualLinesForSegment(line, charactersPerLine),
    0,
  );
  return Math.min(
    maxComposerContentHeight(),
    visualLineCount * INPUT_LINE_HEIGHT,
  );
}

export function shouldExpandComposerHeight(
  contentHeight: number,
  currentlyExpanded: boolean,
): boolean {
  if (currentlyExpanded) {
    return contentHeight > COMPOSER_COLLAPSE_THRESHOLD;
  }
  return contentHeight > COMPOSER_EXPAND_THRESHOLD;
}

/**
 * Merge height samples.
 *
 * Soft-wrap edge: after the first single-line contentSize sample, iOS often
 * keeps reporting ~one line while the view height is clamped. Estimates must
 * still be allowed to *grow* so the field can open.
 *
 * After a real multi-line (or non-clamped) measure, measured is authoritative
 * for shrink. Estimates may still grow to open a soft-wrap stall, but they
 * must not keep the bar ratcheted tall after the draft shortens.
 */
export function resolveNextInputContentHeight(options: {
  current: number;
  next: number;
  source: 'estimate' | 'measured';
  hasMeasured: boolean;
  /**
   * When true, measured content is multi-line / unclamped and owns height.
   * Estimates then only grow (soft-wrap bootstrap), never pin a tall floor.
   */
  measuredAuthoritative?: boolean;
}): number {
  const {
    current,
    next,
    source,
    hasMeasured,
    measuredAuthoritative = false,
  } = options;
  if (next <= 0) return 0;

  if (Math.abs(next - current) <= COMPOSER_HEIGHT_EPSILON) {
    return current;
  }

  if (source === 'estimate') {
    // Always allow estimate growth (soft wrap / paste before remeasure).
    if (next > current) return next;
    // Before measurement, or while still single-line bootstrap, estimates may
    // shrink with the draft. Once multi-line measured content exists, shrink
    // is owned by measured samples (or empty-text reset) — never by estimate.
    if (!hasMeasured || !measuredAuthoritative) return next;
    return current;
  }

  // Measured samples grow freely. Ignore sub-half-line shrinks — those are
  // almost always padding/view remeasure noise between stable line steps.
  if (next < current && current - next < COMPOSER_MEASURED_SHRINK_MIN) {
    return current;
  }
  return next;
}

/**
 * Content-box height of the draft (line stack only, no vertical padding).
 * Used for expansion thresholds and as the basis for the TextInput height.
 */
export function resolveComposerContentHeight(contentHeight: number): number {
  if (contentHeight <= 0) return INPUT_LINE_HEIGHT;
  return Math.min(
    maxComposerContentHeight(),
    Math.max(INPUT_LINE_HEIGHT, contentHeight),
  );
}

/**
 * Height applied to the TextInput view.
 *
 * React Native includes `paddingVertical` inside the explicit `height` box.
 * When the field is expanded we therefore add the vertical padding here so the
 * content-box still fits the measured line stack. Without that, soft wrap
 * opens a 44px-tall field with 8px pad top+bottom and only ~28px of usable
 * text — which looks and feels wrong until a few more lines force it taller.
 *
 * Always follows measured/estimated content so soft-wrapped text can report a
 * true contentSize (and open the composer) instead of being capped at one line
 * while the bar is still collapsed.
 */
export function resolveComposerInputHeight(
  contentHeight: number,
  currentlyExpanded = false,
): number {
  const contentBox = resolveComposerContentHeight(contentHeight);
  const expanded = shouldExpandComposerHeight(contentHeight, currentlyExpanded);
  return contentBox + (expanded ? INPUT_EXPANDED_VERTICAL_PADDING * 2 : 0);
}

/**
 * Resolve the outer chat-bar height from measured input content.
 * Collapsed drafts stay at the pill height. Expanded drafts grow with content.
 * Recording always uses the fixed pill size.
 *
 * Expanded bar = input view height (content + padding) + chrome.
 */
export function resolveComposerBarHeight(
  contentHeight: number,
  isRecording: boolean,
  pillHeight: number = COMPOSER_PILL_HEIGHT,
  currentlyExpanded = false,
): number {
  if (isRecording || !shouldExpandComposerHeight(contentHeight, currentlyExpanded)) {
    return pillHeight;
  }

  return Math.min(
    COMPOSER_MAX_HEIGHT,
    Math.max(
      pillHeight,
      resolveComposerInputHeight(contentHeight, true) + INPUT_GROWTH_CHROME,
    ),
  );
}
