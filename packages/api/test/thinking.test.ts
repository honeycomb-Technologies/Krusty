import type { ModelInfo } from '../src/types.ts';
import {
  cycleThinkingLevel,
  normalizeThinkingLevel,
  selectableThinkingLevels,
  supportsFastMode,
} from '../src/thinking.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

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

Deno.test('reasoning cycle follows model metadata and mandatory default', () => {
  const metadata = model({
    supported_reasoning_levels: ['low', 'high', 'ultra'],
    default_reasoning_level: 'high',
    reasoning_is_mandatory: true,
  });

  assertEquals(
    selectableThinkingLevels(metadata),
    ['low', 'high', 'ultra'],
    'mandatory reasoning must not expose off',
  );
  assertEquals(
    normalizeThinkingLevel('off', metadata),
    'high',
    'an unsupported stored level should use the advertised default',
  );
  assertEquals(
    cycleThinkingLevel('high', metadata),
    'ultra',
    'cycle order must follow the catalog',
  );
});

Deno.test('explicit fast capability overrides provider-name inference', () => {
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
