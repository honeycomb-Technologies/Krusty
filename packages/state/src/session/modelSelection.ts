import type { ModelInfo } from '@krusty/api';

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
): ModelInfo | null {
  for (const modelId of [currentModel, defaultModel]) {
    if (isModelUsable(modelId, catalog, configuredProviders)) {
      return catalog.find((candidate) => candidate.id === modelId) ?? null;
    }
  }

  return (
    catalog.find((candidate) =>
      isModelUsable(candidate.id, catalog, configuredProviders),
    ) ?? null
  );
}
