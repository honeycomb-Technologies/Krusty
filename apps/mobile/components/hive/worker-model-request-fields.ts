import type { ModelInfo, ModelKey } from "@mitsuro/api";

export interface WorkerModelRequestFields {
  model?: string;
  model_key?: ModelKey;
}

function modelKeysEqual(
  left: ModelKey | null | undefined,
  right: ModelKey | null | undefined,
): boolean {
  if (!left || !right) {
    return !left && !right;
  }
  return left.provider === right.provider &&
    left.model_id === right.model_id &&
    (left.auth_scope ?? null) === (right.auth_scope ?? null) &&
    left.api_format === right.api_format;
}

function exactModelFields(
  selectedModel: ModelInfo | null,
): WorkerModelRequestFields | null {
  const key = selectedModel?.key;
  if (!selectedModel || !key) {
    return null;
  }
  return { model: selectedModel.id, model_key: key };
}

/** A new Worker must always persist an exact provider/model identity. */
export function buildWorkerModelCreateFields(
  selectedModel: ModelInfo | null,
): WorkerModelRequestFields | null {
  return exactModelFields(selectedModel);
}

/**
 * An edit with no selected catalog entry leaves the persisted model alone.
 * Keyless legacy catalog entries cannot express an exact change, while an
 * unchanged exact key is omitted so unrelated edits do not revalidate model
 * credentials.
 */
export function buildWorkerModelUpdateFields(
  selectedModel: ModelInfo | null,
  persistedKey: ModelKey | null | undefined,
): WorkerModelRequestFields | null {
  if (!selectedModel) {
    return {};
  }

  const fields = exactModelFields(selectedModel);
  if (!fields?.model_key) {
    return null;
  }
  return modelKeysEqual(fields.model_key, persistedKey) ? {} : fields;
}
