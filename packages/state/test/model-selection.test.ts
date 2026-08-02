import type { ModelInfo, ModelKey } from '@mitsuro/api';
import {
  findModelByKey,
  isModelUsable,
  modelKeysEqual,
  resolveUsableModel,
} from '../src/session/modelSelection.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
}

function model(id: string, provider: string, key?: ModelKey): ModelInfo {
  return {
    key,
    id,
    provider,
    display_name: id,
    context_window: 128_000,
    max_output: 8_192,
    supports_thinking: true,
    supports_tools: true,
    supports_vision: false,
  };
}

const openAiSharedKey: ModelKey = {
  provider: 'openai',
  model_id: 'shared-model',
  auth_scope: 'api_key',
  api_format: 'open_ai_responses',
};
const grokSharedKey: ModelKey = {
  provider: 'grok',
  model_id: 'shared-model',
  auth_scope: 'oauth',
  api_format: 'open_ai_responses',
};

const catalog = [
  model('grok', 'xAI'),
  model('claude', 'Anthropic'),
  model('gpt', 'OpenAI'),
];

Deno.test('model availability normalizes configured provider identifiers', () => {
  assertEquals(
    isModelUsable('grok', catalog, [' XAI ']),
    true,
    'provider matching should be case and whitespace insensitive',
  );
  assertEquals(
    isModelUsable('claude', catalog, ['openai']),
    false,
    'a model without an available provider must not remain selectable for send',
  );
});

Deno.test('model resolution keeps current selection then falls through safely', () => {
  assertEquals(
    resolveUsableModel('gpt', 'claude', catalog, ['openai', 'anthropic'])?.id,
    'gpt',
    'a usable explicit selection wins over the default',
  );
  assertEquals(
    resolveUsableModel('stale-model', 'claude', catalog, ['anthropic'])?.id,
    'claude',
    'a stale selection falls back to the usable server default',
  );
  assertEquals(
    resolveUsableModel('grok', 'claude', catalog, ['openai'])?.id,
    'gpt',
    'when both saved choices are unavailable, select the first send-ready model',
  );
  assertEquals(
    resolveUsableModel('missing', null, catalog, ['google']),
    null,
    'never return a model whose provider cannot send',
  );
});

Deno.test('model resolution preserves exact provider identity for duplicate slugs', () => {
  const sharedCatalog = [
    model('shared-model', 'OpenAI', openAiSharedKey),
    model('shared-model', 'Grok', grokSharedKey),
  ];

  assertEquals(
    modelKeysEqual(findModelByKey(sharedCatalog, grokSharedKey)?.key, grokSharedKey),
    true,
    'exact lookup should retain provider, auth, and transport identity',
  );
  assertEquals(
    resolveUsableModel(
      'shared-model',
      'shared-model',
      sharedCatalog,
      ['openai', 'grok'],
      grokSharedKey,
      openAiSharedKey,
    )?.key?.provider,
    'grok',
    'the current exact key must win over an identical default slug',
  );
  assertEquals(
    resolveUsableModel(
      'missing',
      'shared-model',
      sharedCatalog,
      ['openai', 'grok'],
      null,
      openAiSharedKey,
    )?.key?.provider,
    'openai',
    'the exact server default must select its provider row',
  );
});
