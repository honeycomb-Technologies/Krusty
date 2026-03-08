/**
 * Keyboard layout types and enums
 */

export type KeyboardLayout = 'qwerty' | 'numbers' | 'symbols';

/**
 * Key actions - typed actions for keyboard keys
 */
export enum KeyAction {
	Shift = 'shift',
	Backspace = 'backspace',
	Space = 'space',
	Enter = 'enter',
	SwitchNumbers = 'switch-numbers',
	SwitchSymbols = 'switch-symbols',
	SwitchQwerty = 'switch-qwerty',
}

export interface KeyConfig {
	value: string;
	display?: string;
	width?: 'wide' | 'extra-wide';
	action?: KeyAction;
}
