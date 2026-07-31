/** Composer pill height (matches ChatBar `PILL`). */
export const COMPOSER_PILL_HEIGHT = 56;
export const COMPOSER_MAX_HEIGHT = 112;
export const INPUT_GROWTH_CHROME = 8;
export const INPUT_LINE_HEIGHT = 22;
export const INPUT_EXPANDED_VERTICAL_PADDING = 8;
/**
 * Approximate average glyph width for 16px system UI fonts.
 * Used only as a bootstrap before native contentSize reports.
 * Prefer measured content height once available.
 */
export const COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH = 9.5;
/** Expand once content clearly needs a second visual row. */
export const COMPOSER_EXPAND_THRESHOLD = INPUT_LINE_HEIGHT + 6;
/**
 * Collapse only after content is back near a single centered line.
 * The gap vs expand threshold prevents thrashing around wrap boundaries.
 */
export const COMPOSER_COLLAPSE_THRESHOLD = INPUT_LINE_HEIGHT + 2;
/** Ignore tiny content-size jitter from iOS UITextView measurement noise. */
export const COMPOSER_HEIGHT_EPSILON = 1;

/**
 * Approximate the text block height for composer growth.
 * Bootstrap only: real layout uses measured `onContentSizeChange` height.
 */
export function estimateCompactInputHeight(
  value: string,
  inputWidth: number,
): number {
  if (!value) return 0;
  const usableWidth = Math.max(48, inputWidth);
  const charactersPerLine = Math.max(
    8,
    Math.floor(usableWidth / COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH),
  );
  const visualLineCount = value.split('\n').reduce(
    (total, line) =>
      total + Math.max(1, Math.ceil(Math.max(line.length, 1) / charactersPerLine)),
    0,
  );
  return Math.min(
    COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME,
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
 * Merge a new content-height sample without letting bootstrap estimates
 * yank the field down after a measured multi-line height is known.
 */
export function resolveNextInputContentHeight(options: {
  current: number;
  next: number;
  source: 'estimate' | 'measured';
  hasMeasured: boolean;
}): number {
  const { current, next, source, hasMeasured } = options;
  if (next <= 0) return 0;

  // After a real contentSize sample exists, never let the estimate shrink
  // the bar. Estimates are coarse and fight proportional font wrapping.
  if (source === 'estimate' && hasMeasured) {
    return current;
  }

  if (Math.abs(next - current) <= COMPOSER_HEIGHT_EPSILON) {
    return current;
  }

  // While still estimating, allow both growth and shrink so deletions
  // collapse cleanly before the first measured sample arrives.
  if (source === 'estimate') {
    return next;
  }

  // Measured samples can grow and shrink, but ignore sub-pixel noise.
  return next;
}

/**
 * Height of the multiline TextInput itself.
 * Driven from measured contentSize (text box only; padding is applied in style).
 */
export function resolveComposerInputHeight(
  contentHeight: number,
  currentlyExpanded: boolean,
): number {
  if (!currentlyExpanded || contentHeight <= 0) {
    return INPUT_LINE_HEIGHT;
  }
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

  // Bar = text box + expanded padding + thin chrome. contentHeight is the
  // measured text box only (padding lives in style, not in contentSize).
  const inputHeight = resolveComposerInputHeight(contentHeight, true);
  return Math.min(
    COMPOSER_MAX_HEIGHT,
    Math.max(
      pillHeight,
      inputHeight + INPUT_EXPANDED_VERTICAL_PADDING * 2 + INPUT_GROWTH_CHROME,
    ),
  );
}
