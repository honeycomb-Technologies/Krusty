/**
 * Keyboard layout data - QWERTY, numbers, and symbols layouts
 */

import { KeyAction, type KeyConfig } from './types';

/**
 * Standard QWERTY layout
 */
export const qwertyLayout: KeyConfig[][] = [
	// Row 1
	[
		{ value: 'q' }, { value: 'w' }, { value: 'e' }, { value: 'r' }, { value: 't' },
		{ value: 'y' }, { value: 'u' }, { value: 'i' }, { value: 'o' }, { value: 'p' }
	],
	// Row 2
	[
		{ value: 'a' }, { value: 's' }, { value: 'd' }, { value: 'f' }, { value: 'g' },
		{ value: 'h' }, { value: 'j' }, { value: 'k' }, { value: 'l' }
	],
	// Row 3
	[
		{ value: 'shift', display: '⇧', action: KeyAction.Shift },
		{ value: 'z' }, { value: 'x' }, { value: 'c' }, { value: 'v' }, { value: 'b' },
		{ value: 'n' }, { value: 'm' },
		{ value: 'backspace', display: '⌫', action: KeyAction.Backspace }
	],
	// Row 4
	[
		{ value: '123', display: '123', action: KeyAction.SwitchNumbers, width: 'wide' },
		{ value: ',' },
		{ value: 'space', display: 'space', action: KeyAction.Space, width: 'extra-wide' },
		{ value: 'enter', display: '⏎', action: KeyAction.Enter }
	]
];

/**
 * Numbers layout (alternative to QWERTY)
 */
export const numbersLayout: KeyConfig[][] = [
	// Row 1
	[
		{ value: '1' }, { value: '2' }, { value: '3' }, { value: '4' }, { value: '5' },
		{ value: '6' }, { value: '7' }, { value: '8' }, { value: '9' }, { value: '0' }
	],
	// Row 2
	[
		{ value: '-' }, { value: '/' }, { value: ':' }, { value: ';' }, { value: '(' },
		{ value: ')' }, { value: '$' }, { value: '&' }, { value: '@' }, { value: '"' }
	],
	// Row 3
	[
		{ value: '#+=', display: '#+=', action: KeyAction.SwitchSymbols },
		{ value: '.' }, { value: ',' }, { value: '?' }, { value: '!' }, { value: "'" },
		{ value: 'backspace', display: '⌫', action: KeyAction.Backspace }
	],
	// Row 4
	[
		{ value: 'qwerty', display: 'ABC', action: KeyAction.SwitchQwerty, width: 'wide' },
		{ value: ',' },
		{ value: 'space', display: 'space', action: KeyAction.Space, width: 'extra-wide' },
		{ value: 'enter', display: '⏎', action: KeyAction.Enter }
	]
];

/**
 * Symbols layout (additional characters)
 */
export const symbolsLayout: KeyConfig[][] = [
	// Row 1
	[
		{ value: '[' }, { value: ']' }, { value: '{' }, { value: '}' }, { value: '#' },
		{ value: '%' }, { value: '^' }, { value: '*' }, { value: '+' }, { value: '=' }
	],
	// Row 2
	[
		{ value: '_' }, { value: '\\' }, { value: '|' }, { value: '~' }, { value: '<' },
		{ value: '>' }, { value: '€' }, { value: '£' }, { value: '¥' }, { value: '•' }
	],
	// Row 3
	[
		{ value: '123', display: '123', action: KeyAction.SwitchNumbers },
		{ value: '.' }, { value: ',' }, { value: '?' }, { value: '!' }, { value: "'" },
		{ value: 'backspace', display: '⌫', action: KeyAction.Backspace }
	],
	// Row 4
	[
		{ value: 'qwerty', display: 'ABC', action: KeyAction.SwitchQwerty, width: 'wide' },
		{ value: ',' },
		{ value: 'space', display: 'space', action: KeyAction.Space, width: 'extra-wide' },
		{ value: 'enter', display: '⏎', action: KeyAction.Enter }
	]
];
