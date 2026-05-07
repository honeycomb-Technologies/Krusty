import type { ThinkingLevel } from './types';

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

export function supportsFastMode(
  model: string | null,
  provider?: string | null,
): boolean {
  const providerLower = (provider ?? '').trim().toLowerCase();
  if (providerLower) {
    return (
      providerLower === 'openai'
      || providerLower === 'anthropic'
      || providerLower === 'openrouter'
    );
  }

  const modelLower = (model ?? '').trim().toLowerCase();
  if (!modelLower) return false;

  return (
    modelLower.startsWith('gpt-')
    || modelLower.startsWith('claude-')
    || modelLower.startsWith('openai/')
    || modelLower.startsWith('anthropic/')
  );
}
