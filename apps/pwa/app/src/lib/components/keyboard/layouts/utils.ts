/**
 * Keyboard layout utilities
 */

import type { KeyboardLayout, KeyConfig } from './types';
import { qwertyLayout, numbersLayout, symbolsLayout } from './data';

/**
 * Get layout by type
 */
export function getLayout(type: KeyboardLayout): KeyConfig[][] {
	switch (type) {
		case 'qwerty':
			return qwertyLayout;
		case 'numbers':
			return numbersLayout;
		case 'symbols':
			return symbolsLayout;
		default:
			return qwertyLayout;
	}
}

/**
 * Convert key value to display text
 */
export function formatKeyDisplay(value: string, display?: string): string {
	if (display) return display;
	if (value === 'space') return '';
	return value;
}
