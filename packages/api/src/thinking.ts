import type { ThinkingLevel } from './types';

function supportsExtendedThinkingCycle(model: string | null): boolean {
  const modelLower = (model ?? '').toLowerCase();
  return (
    modelLower.includes('codex')
    || modelLower.startsWith('gpt-5.4')
    || modelLower.startsWith('openai/gpt-5.4')
    || modelLower.startsWith('gpt-5.5')
    || modelLower.startsWith('openai/gpt-5.5')
    || modelLower.includes('opus-4-6')
    || modelLower.includes('opus-4.6')
    || modelLower.includes('opus 4.6')
  );
}

export function cycleThinkingLevel(current: ThinkingLevel, model: string | null): ThinkingLevel {
  if (supportsExtendedThinkingCycle(model)) {
    const cycle: ThinkingLevel[] = ['off', 'low', 'medium', 'high', 'xhigh'];
    const idx = cycle.indexOf(current);
    return cycle[(idx + 1) % cycle.length];
  }

  // Basic toggle for other models
  return current === 'off' ? 'medium' : 'off';
}

export function isThinkingEnabled(level: ThinkingLevel): boolean {
  return level !== 'off';
}

export function thinkingLevelToApiValue(level: ThinkingLevel): string | undefined {
  if (level === 'off') return undefined;
  return level;
}

export function thinkingLevelLabel(level: ThinkingLevel): string {
  switch (level) {
    case 'off': return 'Off';
    case 'low': return 'Low';
    case 'medium': return 'Medium';
    case 'high': return 'High';
    case 'xhigh': return 'Max';
  }
}
