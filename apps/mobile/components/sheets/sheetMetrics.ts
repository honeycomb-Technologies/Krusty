export const APP_SHEET_TOP_GAP = 6;
export const APP_SHEET_MIN_TOP_OFFSET = 18;

export function resolveAppBottomSheetHeight(
  windowHeight: number,
  topInset: number,
): number {
  const topOffset = Math.max(
    Math.max(0, topInset) + APP_SHEET_TOP_GAP,
    APP_SHEET_MIN_TOP_OFFSET,
  );
  return Math.max(1, Math.round(windowHeight - topOffset));
}
