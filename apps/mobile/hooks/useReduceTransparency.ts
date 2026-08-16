import { useSyncExternalStore } from "react";
import {
  AccessibilityInfo,
  type EmitterSubscription,
  Platform,
} from "react-native";

const isIos = Platform.OS === "ios";

// Start conservatively on iOS so accessibility users never see a flash of
// transparent material while the native preference is still loading.
let reduceTransparency = isIos;
let subscription: EmitterSubscription | null = null;
let queriedNativePreference = false;
const listeners = new Set<() => void>();

function publish(nextValue: boolean) {
  if (reduceTransparency === nextValue) return;
  reduceTransparency = nextValue;
  for (const listener of listeners) listener();
}

function queryNativePreference() {
  if (!isIos || queriedNativePreference) return;
  queriedNativePreference = true;

  // A rejected native call must not leave the whole app stuck on the solid
  // fallback; only an affirmative answer keeps transparency reduced.
  void AccessibilityInfo.isReduceTransparencyEnabled()
    .then(publish)
    .catch(() => publish(false));
}

function ensureNativeSubscription() {
  if (!isIos || subscription) return;

  queryNativePreference();
  subscription = AccessibilityInfo.addEventListener(
    "reduceTransparencyChanged",
    publish,
  );
}

// Resolve the preference during module initialization (splash time) so the
// first mounted material already knows the real answer instead of flashing
// solid on every launch.
queryNativePreference();

function subscribe(listener: () => void) {
  listeners.add(listener);
  ensureNativeSubscription();

  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      subscription?.remove();
      subscription = null;
    }
  };
}

function getSnapshot() {
  return reduceTransparency;
}

export function useReduceTransparency(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, () => true);
}
