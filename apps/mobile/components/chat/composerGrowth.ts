/** Composer pill height (matches ChatBar `PILL`). */
export const COMPOSER_PILL_HEIGHT = 56;
export const COMPOSER_MAX_HEIGHT = 112;
export const INPUT_GROWTH_CHROME = 8;
export const INPUT_LINE_HEIGHT = 22;
export const INPUT_EXPANDED_VERTICAL_PADDING = 8;
export const COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH = 8;

/**
 * Approximate the text block height for composer growth.
 * Used as the primary height signal so multi-line growth works even when
 * platform `onContentSizeChange` is delayed, capped, or sticky.
 */
export function estimateCompactInputHeight(
  value: string,
  inputWidth: number,
): number {
  if (!value) return 0;
  const charactersPerLine = Math.max(
    12,
    Math.floor(inputWidth / COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH),
  );
  const visualLineCount = value.split('\n').reduce(
    (total, line) =>
      total + Math.max(1, Math.ceil(line.length / charactersPerLine)),
    0,
  );
  return Math.min(
    COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME,
    visualLineCount * INPUT_LINE_HEIGHT,
  );
}

/** Grow once text needs more than a single centered line inside the pill. */
export function shouldExpandComposerHeight(contentHeight: number): boolean {
  return contentHeight > INPUT_LINE_HEIGHT + 2;
}

export function resolveComposerBarHeight(
  contentHeight: number,
  isRecording: boolean,
  pillHeight: number = COMPOSER_PILL_HEIGHT,
): number {
  if (isRecording || !shouldExpandComposerHeight(contentHeight)) {
    return pillHeight;
  }
  // Expanded mode adds vertical padding; keep the pill as the floor so the
  // bar never shrinks below the FAB row while multi-line text is present.
  return Math.min(
    COMPOSER_MAX_HEIGHT,
    Math.max(
      pillHeight,
      contentHeight + INPUT_GROWTH_CHROME + INPUT_EXPANDED_VERTICAL_PADDING * 2,
    ),
  );
}
