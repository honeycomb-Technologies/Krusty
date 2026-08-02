import type { ChatMessage } from '@mitsuro/api';
import {
  splitTranscriptTurnsCached,
} from '../../../apps/mobile/components/chat/transcriptTurns.ts';

const messages: ChatMessage[] = [];
for (let index = 0; index < 1_000; index += 1) {
  messages.push(
    { id: `u-${index}`, role: 'user', content: `question ${index}` },
    { id: `a-${index}`, role: 'assistant', content: `answer ${index}` },
  );
}
messages[messages.length - 1] = {
  ...messages[messages.length - 1]!,
  kind: 'streaming',
};

const initial = splitTranscriptTurnsCached(messages, true);
let tick = 0;

Deno.bench('transcript 1000 turns - cached live-tail update', () => {
  tick += 1;
  const next = messages.slice();
  next[next.length - 1] = {
    ...next[next.length - 1]!,
    content: `answer 999 token ${tick}`,
  };
  splitTranscriptTurnsCached(next, true, initial.cache);
});

Deno.bench('transcript 1000 turns - full regroup', () => {
  tick += 1;
  const next = messages.slice();
  next[next.length - 1] = {
    ...next[next.length - 1]!,
    content: `answer 999 token ${tick}`,
  };
  splitTranscriptTurnsCached(next, true);
});
