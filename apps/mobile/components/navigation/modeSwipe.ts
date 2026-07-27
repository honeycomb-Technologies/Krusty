import type { SessionType } from "@krusty/api";

const MODE_ORDER: SessionType[] = ["chat", "code", "mako"];
const SWIPE_DISTANCE = 64;
const SWIPE_VELOCITY = 700;

export function modeForHorizontalSwipe(
  currentMode: SessionType,
  translationX: number,
  velocityX: number,
): SessionType | null {
  const intent =
    Math.abs(translationX) >= SWIPE_DISTANCE
      ? translationX
      : Math.abs(velocityX) >= SWIPE_VELOCITY
        ? velocityX
        : 0;
  if (intent === 0) {
    return null;
  }

  const currentIndex = MODE_ORDER.indexOf(currentMode);
  const nextIndex = intent < 0 ? currentIndex + 1 : currentIndex - 1;
  return MODE_ORDER[nextIndex] ?? null;
}
