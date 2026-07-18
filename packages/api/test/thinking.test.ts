import { test } from 'bun:test';
import type { ModelInfo } from '../src/types.ts';
import {
  cycleThinkingLevel,
  normalizeThinkingLevel,
  selectableThinkingLevels,
  supportsFastMode,
  supportsThinking,
} from '../src/thinking.ts';

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${JSON.stringify(actual)}`);
  }
}

function model(overrides: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id: 'gpt-test',
    display_name: 'GPT Test',
    provider: 'OpenAI',
    context_window: 128_000,
    max_output: 16_384,
    supports_thinking: true,
    supports_tools: true,
    supports_vision: true,
    ...overrides,
  };
}

test('reasoning cycle follows controllable metadata and omits delegation-only ultra', () => {
  const metadata = model({
    supported_reasoning_levels: ['low', 'high', 'ultra'],
    default_reasoning_level: 'high',
    reasoning_is_mandatory: true,
  });

  assertEquals(
    selectableThinkingLevels(metadata),
    ['low', 'high'],
    'mandatory reasoning must not expose off or delegation-only ultra',
  );
  assertEquals(
    normalizeThinkingLevel('off', metadata),
    'high',
    'an unsupported stored level should use the advertised default',
  );
  assertEquals(
    cycleThinkingLevel('high', metadata),
    'low',
    'cycle order must follow the catalog',
  );
});

test('output-only and malformed mandatory metadata stay non-interactive and safe', () => {
  const outputOnly = model({
    reasoning_control: 'output_only',
    supported_reasoning_levels: [],
  });
  assertEquals(
    selectableThinkingLevels(outputOnly),
    ['off'],
    'output-only reasoning must not expose an explicit control',
  );
  assertEquals(
    supportsThinking(outputOnly),
    false,
    'output-only reasoning is observable but not user-controllable',
  );

  const mandatoryWithoutEnabledLevel = model({
    reasoning_control: 'open_ai_effort',
    supported_reasoning_levels: ['none'],
    default_reasoning_level: null,
    reasoning_is_mandatory: true,
  });
  assertEquals(
    selectableThinkingLevels(mandatoryWithoutEnabledLevel),
    ['medium'],
    'mandatory cycles must retain a safe non-off fallback',
  );
});

test('explicit fast capability overrides provider-name inference', () => {
  assertEquals(
    supportsFastMode(model({ supports_fast_mode: false, fast_mode: null })),
    false,
    'an explicit false capability must win even for OpenAI',
  );
  assertEquals(
    supportsFastMode(model({ supports_fast_mode: true, fast_mode: 'priority' })),
    true,
    'the catalog should enable its advertised fast mode',
  );
  assertEquals(
    supportsFastMode('gpt-legacy', 'openai'),
    true,
    'older callers without model metadata retain a safe fallback',
  );
});
