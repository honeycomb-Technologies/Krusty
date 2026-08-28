export type ModelPreferencePolicy =
  | "shared"
  | "default-only"
  | "store-only";

export function resolveModelPreferencePolicy(
  targetsHiveStore: boolean,
  hiveSessionId: string | null,
): ModelPreferencePolicy {
  if (!targetsHiveStore) return "shared";
  return hiveSessionId == null ? "default-only" : "store-only";
}
