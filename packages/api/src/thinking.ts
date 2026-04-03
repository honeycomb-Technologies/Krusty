import type { ThinkingLevel } from './types';

export function cycleThinkingLevel(current: ThinkingLevel, model: string | null): ThinkingLevel {
  const modelLower = (model ?? '').toLowerCase();
  const isCodex = modelLower.includes('codex');
  const isOpus = modelLower.includes('opus');

  if (isCodex) {
    // Full cycle for Codex models
    const cycle: ThinkingLevel[] = ['off', 'low', 'medium', 'high', 'xhigh'];
    const idx = cycle.indexOf(current);
    return cycle[(idx + 1) % cycle.length];
  }

  if (isOpus) {
    // Limited cycle for Opus models
    const cycle: ThinkingLevel[] = ['off', 'low', 'medium', 'high'];
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
