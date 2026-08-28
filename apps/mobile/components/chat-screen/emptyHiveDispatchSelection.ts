import type { ModelKey } from "@mitsuro/api";

export interface EmptyHiveDispatchSelection {
  model: string;
  modelKey?: ModelKey;
}

export function buildEmptyHiveDispatchSelection(
  model: string,
  modelKey: ModelKey | null,
): EmptyHiveDispatchSelection {
  return {
    model,
    modelKey: modelKey ?? undefined,
  };
}
