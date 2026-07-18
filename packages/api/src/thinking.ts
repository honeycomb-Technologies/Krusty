import type { ModelInfo, ReasoningEffort, ThinkingLevel } from './types';

export type ModelCapabilityInput = ModelInfo | string | null | undefined;

function isModelInfo(value: ModelCapabilityInput): value is ModelInfo {
  return typeof value === 'object' && value !== null && typeof value.id === 'string';
}

function toThinkingLevel(level: ReasoningEffort | null | undefined): ThinkingLevel | null {
  switch (level) {
    case 'none': return 'off';
    case 'minimal':
    case 'low':
    case 'medium':
    case 'high':
    case 'xhigh':
    case 'max':
    case 'ultra':
      return level;
    default:
      return null;
  }
}

function legacyThinkingLevels(modelId: string | null | undefined): ThinkingLevel[] {
  const modelLower = (modelId ?? '').toLowerCase();
  const supportsExtendedCycle =
    modelLower.includes('codex')
    || modelLower.startsWith('gpt-5.4')
    || modelLower.startsWith('openai/gpt-5.4')
    || modelLower.startsWith('gpt-5.5')
    || modelLower.startsWith('openai/gpt-5.5')
    || modelLower.startsWith('gpt-5.6')
    || modelLower.startsWith('openai/gpt-5.6')
    || modelLower.includes('opus-4-6')
    || modelLower.includes('opus-4.6')
    || modelLower.includes('opus 4.6');

  return supportsExtendedCycle
    ? ['off', 'low', 'medium', 'high', 'xhigh']
    : ['off', 'medium'];
}

/** Return the exact UI cycle advertised by a model, with a legacy fallback. */
export function selectableThinkingLevels(model: ModelCapabilityInput): ThinkingLevel[] {
  if (!isModelInfo(model)) {
    return legacyThinkingLevels(model);
  }
  if (model.reasoning_control === 'output_only') {
    return ['off'];
  }

  const advertised = (model.supported_reasoning_levels ?? [])
    .map(toThinkingLevel)
    .filter((level): level is ThinkingLevel => level !== null && level !== 'ultra');
  const levels = [...new Set(advertised)];
  const defaultLevel = toThinkingLevel(model.default_reasoning_level);
  const fallback = defaultLevel && defaultLevel !== 'off' && defaultLevel !== 'ultra'
    ? defaultLevel
    : legacyThinkingLevels(model.id).find((level) => level !== 'off') ?? 'medium';

  if (levels.length === 0) {
    if (!model.supports_thinking) return ['off'];
    return model.reasoning_is_mandatory
      ? [fallback]
      : ['off', fallback];
  }

  if (model.reasoning_is_mandatory) {
    const mandatoryLevels = levels.filter((level) => level !== 'off');
    return mandatoryLevels.length > 0 ? mandatoryLevels : [fallback];
  }
  return levels.includes('off') ? levels : ['off', ...levels];
}

/** Clamp a stored/requested level to the selected model's advertised controls. */
export function normalizeThinkingLevel(
  current: ThinkingLevel,
  model: ModelCapabilityInput,
): ThinkingLevel {
  const levels = selectableThinkingLevels(model);
  if (levels.includes(current)) return current;

  if (isModelInfo(model)) {
    const defaultLevel = toThinkingLevel(model.default_reasoning_level);
    if (defaultLevel && levels.includes(defaultLevel)) return defaultLevel;
  }
  return levels[0] ?? 'off';
}

export function cycleThinkingLevel(
  current: ThinkingLevel,
  model: ModelCapabilityInput,
): ThinkingLevel {
  const levels = selectableThinkingLevels(model);
  const index = levels.indexOf(current);
  if (index < 0) return normalizeThinkingLevel(current, model);
  return levels[(index + 1) % levels.length] ?? 'off';
}

export function supportsThinking(model: ModelCapabilityInput): boolean {
  if (!isModelInfo(model)) return true;
  if (model.reasoning_control === 'output_only') return false;
  return model.supports_thinking
    || (model.supported_reasoning_levels ?? []).some(
      (level) => level !== 'none' && level !== 'ultra',
    );
}

export function supportsFastMode(
  model: ModelCapabilityInput,
  provider?: string | null,
): boolean {
  if (isModelInfo(model)) {
    if (typeof model.supports_fast_mode === 'boolean') return model.supports_fast_mode;
    if (model.fast_mode != null) return true;
    provider = model.provider;
    model = model.id;
  }

  // Backward-compatible inference for older servers/callers without metadata.
  const providerLower = (provider ?? '').trim().toLowerCase();
  if (providerLower) {
    return (
      providerLower === 'openai'
      || providerLower === 'anthropic'
      || providerLower === 'openrouter'
    );
  }
  const modelLower = (model ?? '').trim().toLowerCase();
  return Boolean(modelLower) && (
    modelLower.startsWith('gpt-')
    || modelLower.startsWith('claude-')
    || modelLower.startsWith('openai/')
    || modelLower.startsWith('anthropic/')
  );
}

export function isThinkingEnabled(level: ThinkingLevel): boolean {
  return level !== 'off';
}

export function thinkingLevelToApiValue(level: ThinkingLevel): ThinkingLevel | undefined {
  return level === 'off' ? undefined : level;
}

export function thinkingLevelLabel(level: ThinkingLevel): string {
  switch (level) {
    case 'off': return 'Off';
    case 'minimal': return 'Minimal';
    case 'low': return 'Low';
    case 'medium': return 'Medium';
    case 'high': return 'High';
    case 'xhigh': return 'Extra High';
    case 'max': return 'Max';
    case 'ultra': return 'Ultra';
  }
}
