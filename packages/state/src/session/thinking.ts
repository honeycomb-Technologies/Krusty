import {
  cycleThinkingLevel as cycleApiThinkingLevel,
  isThinkingEnabled,
  normalizeThinkingLevel as normalizeApiThinkingLevel,
  supportsFastMode,
  supportsThinking,
  thinkingLevelLabel,
  thinkingLevelToApiValue,
} from '@krusty/api';
import type { ModelCapabilityInput } from '@krusty/api';
import type { ThinkingLevel } from './types';

export { isThinkingEnabled, supportsFastMode, supportsThinking, thinkingLevelLabel, thinkingLevelToApiValue };

export function cycleThinkingLevel(
  current: ThinkingLevel,
  model: ModelCapabilityInput,
): ThinkingLevel {
  return cycleApiThinkingLevel(current, model);
}

export function normalizeThinkingLevel(
  current: ThinkingLevel,
  model: ModelCapabilityInput,
): ThinkingLevel {
  return normalizeApiThinkingLevel(current, model);
}
