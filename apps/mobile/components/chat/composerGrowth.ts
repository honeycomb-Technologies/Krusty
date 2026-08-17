/** Composer pill height (matches ChatBar `PILL`). */
export const COMPOSER_PILL_HEIGHT = 56;
export const COMPOSER_MAX_HEIGHT = 112;
export const INPUT_GROWTH_CHROME = 8;
export const INPUT_LINE_HEIGHT = 22;
/** Visual breathing room owned by the outer bar, never the TextInput. */
export const INPUT_EXPANDED_VERTICAL_PADDING = 8;

/**
 * Approximate average glyph width for the 16px system font used by ChatBar.
 * This estimate is intentionally conservative so a soft wrap opens the second
 * row slightly early instead of clipping typed text behind a one-line field.
 */
export const COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH = 8.2;
export const COMPACT_INPUT_WORD_WRAP_FACTOR = 0.92;

export interface ComposerLayout {
  contentHeight: number;
  inputHeight: number;
  barHeight: number;
  expanded: boolean;
  scrollEnabled: boolean;
}

/** Maximum text stack that fits inside the capped composer. */
export function maxComposerContentHeight(): number {
  return (
    COMPOSER_MAX_HEIGHT -
    INPUT_GROWTH_CHROME -
    INPUT_EXPANDED_VERTICAL_PADDING * 2
  );
}

export function maxComposerVisibleLines(): number {
  return Math.max(
    1,
    Math.floor(maxComposerContentHeight() / INPUT_LINE_HEIGHT),
  );
}

/**
 * Estimate visual lines for one hard-break segment.
 *
 * This is deliberately bounded by `maxLines`: once the composer is full, more
 * draft text cannot change its layout and must not make every keystroke scan a
 * potentially huge draft.
 */
export function countVisualLinesForSegment(
  segment: string,
  charactersPerLine: number,
  maxLines = Number.POSITIVE_INFINITY,
): number {
  const cpl = Math.max(8, charactersPerLine);
  const limit = Math.max(1, maxLines);
  if (!segment) return 1;

  // A segment this long necessarily fills the bounded composer. Avoid tokenizing
  // or scanning the rest of a large pasted draft.
  if (Number.isFinite(limit) && segment.length >= cpl * limit) {
    return limit;
  }

  const words = segment.split(/(\s+)/);
  let lines = 1;
  let column = 0;

  for (const token of words) {
    if (!token) continue;

    if (/^\s+$/.test(token)) {
      if (column > 0 && column + token.length > cpl) {
        lines += 1;
        column = 0;
      } else {
        column += token.length;
      }
    } else if (token.length > cpl) {
      const available = column > 0 ? cpl - column : cpl;
      let remaining = token.length;
      if (column > 0 && remaining > available) {
        remaining -= available;
        lines += 1;
        column = 0;
      }
      if (remaining > 0) {
        lines += Math.floor((remaining - 1) / cpl);
        column = ((remaining - 1) % cpl) + 1;
      }
    } else if (column > 0 && column + token.length > cpl) {
      lines += 1;
      column = token.length;
    } else {
      column += token.length;
    }

    if (lines >= limit) return limit;
  }

  return Math.max(1, Math.min(lines, limit));
}

/**
 * Deterministic, bounded content height for typed and pasted drafts.
 *
 * Native `contentSize` is intentionally not fed back into TextInput height.
 * Doing so created a cycle where changing the explicit height emitted another
 * measurement, which changed height again around the second/third line.
 */
export function estimateCompactInputHeight(
  value: string,
  inputWidth: number,
): number {
  if (!value) return 0;

  const usableWidth = Math.max(48, inputWidth);
  const rawCapacity = Math.floor(
    usableWidth / COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH,
  );
  const charactersPerLine = Math.max(
    8,
    Math.floor(rawCapacity * COMPACT_INPUT_WORD_WRAP_FACTOR),
  );
  const maxLines = maxComposerVisibleLines();

  // Once raw character capacity fills every visible line, exact word packing
  // cannot change the capped layout.
  if (value.length >= charactersPerLine * maxLines) {
    return maxLines * INPUT_LINE_HEIGHT;
  }

  let visualLines = 0;
  for (const segment of value.split("\n")) {
    visualLines += countVisualLinesForSegment(
      segment,
      charactersPerLine,
      maxLines - visualLines,
    );
    if (visualLines >= maxLines) {
      visualLines = maxLines;
      break;
    }
  }

  return Math.max(1, visualLines) * INPUT_LINE_HEIGHT;
}

export function shouldExpandComposerHeight(contentHeight: number): boolean {
  return contentHeight > INPUT_LINE_HEIGHT;
}

export function resolveComposerContentHeight(contentHeight: number): number {
  if (contentHeight <= 0) return INPUT_LINE_HEIGHT;
  return Math.min(
    maxComposerContentHeight(),
    Math.max(INPUT_LINE_HEIGHT, Math.ceil(contentHeight)),
  );
}

/** TextInput owns text height only; the outer bar owns all visual inset. */
export function resolveComposerInputHeight(contentHeight: number): number {
  return resolveComposerContentHeight(contentHeight);
}

export function resolveComposerBarHeight(
  contentHeight: number,
  isRecording: boolean,
  pillHeight: number = COMPOSER_PILL_HEIGHT,
): number {
  if (isRecording || !shouldExpandComposerHeight(contentHeight)) {
    return pillHeight;
  }

  return Math.min(
    COMPOSER_MAX_HEIGHT,
    Math.max(
      pillHeight,
      resolveComposerInputHeight(contentHeight) +
        INPUT_EXPANDED_VERTICAL_PADDING * 2 +
        INPUT_GROWTH_CHROME,
    ),
  );
}

/** One pure layout contract consumed by ChatBar and its regression tests. */
export function resolveComposerLayout(
  value: string,
  inputWidth: number,
  isRecording = false,
): ComposerLayout {
  const contentHeight = estimateCompactInputHeight(value, inputWidth);
  const expanded = shouldExpandComposerHeight(contentHeight);
  return {
    contentHeight,
    inputHeight: resolveComposerInputHeight(contentHeight),
    barHeight: resolveComposerBarHeight(
      contentHeight,
      isRecording,
      COMPOSER_PILL_HEIGHT,
    ),
    expanded,
    scrollEnabled: contentHeight >= maxComposerContentHeight() &&
      value.length > 0,
  };
}
