import { shouldAnimateComposerPulse } from '../components/chat/composerPulsePolicy';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('local composer activity never starts the agent Pulse', () => {
  for (const state of [
    { inputFocused: true, expandedEditorOpen: false, hasDraft: false, hasAttachments: false },
    { inputFocused: false, expandedEditorOpen: true, hasDraft: false, hasAttachments: false },
    { inputFocused: true, expandedEditorOpen: false, hasDraft: true, hasAttachments: false },
    { inputFocused: false, expandedEditorOpen: false, hasDraft: false, hasAttachments: true },
  ]) {
    assert(
      !shouldAnimateComposerPulse({ isStreaming: false, ...state }),
      'focus, drafts, editor state, and attachments must remain idle',
    );
  }
});

Deno.test('agent execution starts the Pulse regardless of composer state', () => {
  assert(
    shouldAnimateComposerPulse({
      isStreaming: true,
      inputFocused: true,
      expandedEditorOpen: false,
      hasDraft: true,
      hasAttachments: false,
    }),
    'streaming agent work should animate the Pulse',
  );
});
