import type { ModelInfo, ModelKey } from '@mitsuro/api';

export function modelKeysEqual(
  left: ModelKey | null | undefined,
  right: ModelKey | null | undefined,
): boolean {
  if (!left || !right) return !left && !right;
  return left.provider === right.provider
    && left.model_id === right.model_id
    && (left.auth_scope ?? null) === (right.auth_scope ?? null)
    && left.api_format === right.api_format;
}

export function findModelByKey(
  catalog: ModelInfo[],
  key: ModelKey | null | undefined,
): ModelInfo | null {
  if (!key) return null;
  return catalog.find((candidate) => modelKeysEqual(candidate.key, key)) ?? null;
}

export function normalizeProviderId(
  provider: string | null | undefined,
): string {
  return (provider ?? '').trim().toLowerCase();
}

export function isModelUsable(
  modelId: string | null | undefined,
  catalog: ModelInfo[],
  configuredProviders: string[],
): boolean {
  if (!modelId) return false;
  const match = catalog.find((candidate) => candidate.id === modelId);
  if (!match) return false;
  if (configuredProviders.length === 0) return true;

  const allowed = new Set(configuredProviders.map(normalizeProviderId));
  return allowed.has(normalizeProviderId(match.provider));
}

/** Resolve a send-ready model from shared catalog and credential state. */
export function resolveUsableModel(
  currentModel: string | null | undefined,
  defaultModel: string | null | undefined,
  catalog: ModelInfo[],
  configuredProviders: string[],
  currentModelKey?: ModelKey | null,
  defaultModelKey?: ModelKey | null,
): ModelInfo | null {
  const exactCurrent = findModelByKey(catalog, currentModelKey);
  if (
    exactCurrent
    && isModelUsable(exactCurrent.id, [exactCurrent], configuredProviders)
  ) {
    return exactCurrent;
  }
  if (!currentModelKey && isModelUsable(currentModel, catalog, configuredProviders)) {
    return catalog.find((candidate) => candidate.id === currentModel) ?? null;
  }

  const exactDefault = findModelByKey(catalog, defaultModelKey);
  if (
    exactDefault
    && isModelUsable(exactDefault.id, [exactDefault], configuredProviders)
  ) {
    return exactDefault;
  }
  if (!defaultModelKey && isModelUsable(defaultModel, catalog, configuredProviders)) {
    return catalog.find((candidate) => candidate.id === defaultModel) ?? null;
  }

  return (
    catalog.find((candidate) =>
      isModelUsable(candidate.id, catalog, configuredProviders),
    ) ?? null
  );
}
