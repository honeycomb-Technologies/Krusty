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
const listeners = new Set<() => void>();

function publish(nextValue: boolean) {
  if (reduceTransparency === nextValue) return;
  reduceTransparency = nextValue;
  for (const listener of listeners) listener();
}

function ensureNativeSubscription() {
  if (!isIos || subscription) return;

  void AccessibilityInfo.isReduceTransparencyEnabled().then(publish);
  subscription = AccessibilityInfo.addEventListener(
    "reduceTransparencyChanged",
    publish,
  );
}

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
