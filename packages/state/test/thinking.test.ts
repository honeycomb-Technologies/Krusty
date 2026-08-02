import type { ModelInfo } from '@mitsuro/api';
import {
  cycleThinkingLevel,
  normalizeThinkingLevel,
  supportsFastMode,
} from '../src/session/thinking.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
}

const model: ModelInfo = {
  id: 'claude-test',
  display_name: 'Claude Test',
  provider: 'Anthropic',
  context_window: 200_000,
  max_output: 32_000,
  supports_thinking: true,
  supported_reasoning_levels: ['low', 'medium', 'high', 'max'],
  default_reasoning_level: 'medium',
  reasoning_is_mandatory: false,
  supports_fast_mode: false,
  fast_mode: null,
  supports_tools: true,
  supports_vision: true,
};

Deno.test('state reasoning helpers honor selected model capabilities', () => {
  assertEquals(
    normalizeThinkingLevel('ultra', model),
    'medium',
    'unsupported levels should clamp to the model default',
  );
  assertEquals(
    cycleThinkingLevel('high', model),
    'max',
    'state cycle should follow the advertised order',
  );
  assertEquals(
    supportsFastMode(model),
    false,
    'explicit model capability should override provider inference',
  );
});
