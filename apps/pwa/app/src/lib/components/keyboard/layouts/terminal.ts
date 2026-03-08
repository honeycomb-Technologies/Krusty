/**
 * Terminal-specific keyboard toolbar and escape sequences
 */

import { type KeyConfig } from './types';

/**
 * Terminal toolbar — rendered above the main keyboard in terminal mode.
 * Single row with essential terminal controls.
 */
export const terminalToolbar: KeyConfig[] = [
	{ value: 'escape', display: 'Esc' },
	{ value: 'tab', display: 'Tab' },
	{ value: 'ctrl', display: 'Ctrl' },
	{ value: 'up', display: '↑' },
	{ value: 'down', display: '↓' },
	{ value: 'left', display: '←' },
	{ value: 'right', display: '→' },
	{ value: '|' },
];

/**
 * Terminal key to xterm/ANSI escape sequence mapping
 */
export function getTerminalSequence(key: string): string {
	const sequences: Record<string, string> = {
		// Navigation
		up: '\x1b[A',
		down: '\x1b[B',
		right: '\x1b[C',
		left: '\x1b[D',
		
		// Function keys
		escape: '\x1b',
		tab: '\t',
		
		// Ctrl combinations (simplified - would need modifier handling)
		ctrl: '',
		
		// Special
		backspace: '\x7f',
		enter: '\r',
		space: ' ',
	};
	
	return sequences[key] || key;
}
