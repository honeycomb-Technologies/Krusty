export interface ComposerPulseState {
  isStreaming: boolean;
  inputFocused: boolean;
  expandedEditorOpen: boolean;
  hasDraft: boolean;
  hasAttachments: boolean;
}

/** Pulse communicates agent execution, never local composer activity. */
export function shouldAnimateComposerPulse({
  isStreaming,
}: ComposerPulseState): boolean {
  return isStreaming;
}
