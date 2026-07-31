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
    COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME - INPUT_EXPANDED_VERTICAL_PADDING * 2,
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
 * still be allowed to *grow* so the field can open; they must not *shrink*
 * after measurement (that reintroduced thrash).
 */
export function resolveNextInputContentHeight(options: {
  current: number;
  next: number;
  source: 'estimate' | 'measured';
  hasMeasured: boolean;
}): number {
  const { current, next, source, hasMeasured } = options;
  if (next <= 0) return 0;

  if (Math.abs(next - current) <= COMPOSER_HEIGHT_EPSILON) {
    return current;
  }

  if (source === 'estimate') {
    // Always allow estimate growth (soft wrap / paste before remeasure).
    if (next > current) return next;
    // Estimates may shrink only before a real measurement exists.
    if (!hasMeasured) return next;
    return current;
  }

  // Measured samples grow and shrink (with epsilon already applied).
  return next;
}

/**
 * Height of the multiline TextInput text box (padding applied in style).
 * Always tracks content — never force a one-line clamp while text is taller.
 * Clamping here is what made soft-wrap contentSize stick and hide text.
 */
export function resolveComposerInputHeight(contentHeight: number): number {
  if (contentHeight <= 0) return INPUT_LINE_HEIGHT;
  return Math.min(
    COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME - INPUT_EXPANDED_VERTICAL_PADDING * 2,
    Math.max(INPUT_LINE_HEIGHT, contentHeight),
  );
}

export function resolveComposerBarHeight(
  contentHeight: number,
  isRecording: boolean,
  pillHeight: number = COMPOSER_PILL_HEIGHT,
  currentlyExpanded = false,
): number {
  if (isRecording || !shouldExpandComposerHeight(contentHeight, currentlyExpanded)) {
    return pillHeight;
  }

  const inputHeight = resolveComposerInputHeight(contentHeight);
  return Math.min(
    COMPOSER_MAX_HEIGHT,
    Math.max(
      pillHeight,
      inputHeight + INPUT_EXPANDED_VERTICAL_PADDING * 2 + INPUT_GROWTH_CHROME,
    ),
  );
}
