import type { ThinkingLevel } from './types';

interface FastModelPair {
  standard: string;
  fast: string;
}

const FAST_MODEL_PAIRS: FastModelPair[] = [
  { standard: 'gpt-5.5', fast: 'gpt-5.5-mini' },
  { standard: 'gpt-5.4', fast: 'gpt-5.4-mini' },
  { standard: 'claude-opus-4-6', fast: 'claude-haiku-4-5-20251001' },
  { standard: 'claude-opus-4.6', fast: 'claude-haiku-4.5' },
  { standard: 'anthropic/claude-opus-4.6', fast: 'anthropic/claude-haiku-4.5' },
];

export function isThinkingEnabled(level: ThinkingLevel): boolean {
  return level !== 'off';
}

export function cycleThinkingLevel(
  current: ThinkingLevel,
  model: string | null,
): ThinkingLevel {
  const modelLower = (model ?? '').toLowerCase();
  const supportsExtendedCycle =
    modelLower.includes('codex')
    || modelLower.startsWith('gpt-5.4')
    || modelLower.startsWith('openai/gpt-5.4')
    || modelLower.startsWith('gpt-5.5')
    || modelLower.startsWith('openai/gpt-5.5')
    || modelLower.includes('opus-4-6')
    || modelLower.includes('opus-4.6')
    || modelLower.includes('opus 4.6');

  if (supportsExtendedCycle) {
    switch (current) {
      case 'off':
        return 'low';
      case 'low':
        return 'medium';
      case 'medium':
        return 'high';
      case 'high':
        return 'xhigh';
      case 'xhigh':
        return 'off';
    }
  }

  if (current === 'off') return 'medium';
  return 'off';
}

export function thinkingLevelToApiValue(
  level: ThinkingLevel,
): string | undefined {
  if (level === 'off') return undefined;
  switch (level) {
    case 'low':
      return 'low';
    case 'medium':
      return 'medium';
    case 'high':
      return 'high';
    case 'xhigh':
      return 'xhigh';
  }
  return undefined;
}

export function thinkingLevelLabel(level: ThinkingLevel): string {
  return level;
}

function findFastModelPair(model: string | null): FastModelPair | undefined {
  if (!model) return undefined;
  return FAST_MODEL_PAIRS.find(
    (pair) => pair.standard === model || pair.fast === model,
  );
}

export function supportsFastMode(model: string | null): boolean {
  return findFastModelPair(model) !== undefined;
}

export function isFastModeModel(model: string | null): boolean {
  const pair = findFastModelPair(model);
  return pair !== undefined && pair.fast === model;
}

export function toggleFastModeModel(model: string | null): string | null {
  const pair = findFastModelPair(model);
  if (!pair || !model) return model;
  return pair.fast === model ? pair.standard : pair.fast;
}
