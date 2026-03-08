/**
 * Virtual Keyboard Layout Tests
 * 
 * Run with: npx vitest run
 * Install vitest: npm install -D vitest
 */

import { describe, it, expect } from 'vitest';
import { getLayout, formatKeyDisplay, getTerminalSequence, qwertyLayout, numbersLayout, symbolsLayout, terminalToolbar, KeyAction } from './layouts';

describe('Keyboard Layouts', () => {
	describe('getLayout', () => {
		it('should return qwerty layout for qwerty type', () => {
			const layout = getLayout('qwerty');
			expect(layout).toBe(qwertyLayout);
		});

		it('should return numbers layout for numbers type', () => {
			const layout = getLayout('numbers');
			expect(layout).toBe(numbersLayout);
		});

		it('should return symbols layout for symbols type', () => {
			const layout = getLayout('symbols');
			expect(layout).toBe(symbolsLayout);
		});

		it('should return qwerty layout for unknown types', () => {
			// @ts-expect-error - testing invalid input
			const layout = getLayout('unknown');
			expect(layout).toBe(qwertyLayout);
		});
	});

	describe('formatKeyDisplay', () => {
		it('should return display value when provided', () => {
			expect(formatKeyDisplay('q', 'Q')).toBe('Q');
		});

		it('should return empty string for space', () => {
			expect(formatKeyDisplay('space')).toBe('');
		});

		it('should return value when no display provided', () => {
			expect(formatKeyDisplay('q')).toBe('q');
			expect(formatKeyDisplay('1')).toBe('1');
		});
	});

	describe('qwertyLayout', () => {
		it('should have 4 rows', () => {
			expect(qwertyLayout).toHaveLength(4);
		});

		it('should have shift key in row 3', () => {
			const row3 = qwertyLayout[2];
			expect(row3[0].action).toBe(KeyAction.Shift);
		});

		it('should have backspace key in row 3', () => {
			const row3 = qwertyLayout[2];
			const lastKey = row3[row3.length - 1];
			expect(lastKey.action).toBe(KeyAction.Backspace);
		});

		it('should have space key in row 4', () => {
			const row4 = qwertyLayout[3];
			const spaceKey = row4.find(k => k.action === KeyAction.Space);
			expect(spaceKey).toBeDefined();
			expect(spaceKey?.width).toBe('extra-wide');
		});

		it('should have enter key in row 4', () => {
			const row4 = qwertyLayout[3];
			const enterKey = row4.find(k => k.action === KeyAction.Enter);
			expect(enterKey).toBeDefined();
		});
	});

	describe('numbersLayout', () => {
		it('should have 4 rows', () => {
			expect(numbersLayout).toHaveLength(4);
		});

		it('should have digit keys in row 1', () => {
			const row1 = numbersLayout[0];
			expect(row1.map(k => k.value).join('')).toBe('1234567890');
		});
	});

	describe('terminalToolbar', () => {
		it('should be a flat array of keys', () => {
			expect(terminalToolbar.length).toBeGreaterThan(0);
			expect(terminalToolbar[0]).toHaveProperty('value');
		});

		it('should have navigation keys', () => {
			const values = terminalToolbar.map(k => k.value);
			expect(values).toContain('up');
			expect(values).toContain('down');
			expect(values).toContain('left');
			expect(values).toContain('right');
		});

		it('should have terminal control keys', () => {
			const values = terminalToolbar.map(k => k.value);
			expect(values).toContain('escape');
			expect(values).toContain('tab');
			expect(values).toContain('ctrl');
		});
	});

	describe('KeyAction enum', () => {
		it('should have correct values', () => {
			expect(KeyAction.Shift).toBe('shift');
			expect(KeyAction.Backspace).toBe('backspace');
			expect(KeyAction.Space).toBe('space');
			expect(KeyAction.Enter).toBe('enter');
			expect(KeyAction.SwitchNumbers).toBe('switch-numbers');
			expect(KeyAction.SwitchSymbols).toBe('switch-symbols');
			expect(KeyAction.SwitchQwerty).toBe('switch-qwerty');
		});
	});
});

describe('Terminal Key Sequences', () => {
	describe('getTerminalSequence', () => {
		it('should return up arrow escape sequence', () => {
			expect(getTerminalSequence('up')).toBe('\x1b[A');
		});

		it('should return down arrow escape sequence', () => {
			expect(getTerminalSequence('down')).toBe('\x1b[B');
		});

		it('should return right arrow escape sequence', () => {
			expect(getTerminalSequence('right')).toBe('\x1b[C');
		});

		it('should return left arrow escape sequence', () => {
			expect(getTerminalSequence('left')).toBe('\x1b[D');
		});

		it('should return escape character for escape key', () => {
			expect(getTerminalSequence('escape')).toBe('\x1b');
		});

		it('should return tab character for tab key', () => {
			expect(getTerminalSequence('tab')).toBe('\t');
		});

		it('should return backspace character', () => {
			expect(getTerminalSequence('backspace')).toBe('\x7f');
		});

		it('should return carriage return for enter', () => {
			expect(getTerminalSequence('enter')).toBe('\r');
		});

		it('should return space for space key', () => {
			expect(getTerminalSequence('space')).toBe(' ');
		});

		it('should return empty string for ctrl', () => {
			expect(getTerminalSequence('ctrl')).toBe('');
		});

		it('should return unknown keys as-is', () => {
			expect(getTerminalSequence('ls')).toBe('ls');
			expect(getTerminalSequence('cd')).toBe('cd');
			expect(getTerminalSequence('test')).toBe('test');
		});
	});
});

/**
 * Integration Test: Terminal Toolbar Key Sequence Generation
 *
 * This test verifies that terminal toolbar keys generate the correct
 * ANSI escape sequences when combined with getTerminalSequence.
 */
describe('Terminal Toolbar Integration', () => {
	it('should generate correct escape sequences for all toolbar navigation keys', () => {
		const expected: Record<string, string> = {
			up: '\x1b[A',
			down: '\x1b[B',
			left: '\x1b[D',
			right: '\x1b[C',
			escape: '\x1b',
			tab: '\t',
		};

		for (const [value, sequence] of Object.entries(expected)) {
			const key = terminalToolbar.find(k => k.value === value);
			expect(key, `toolbar should contain ${value}`).toBeDefined();
			expect(getTerminalSequence(key!.value)).toBe(sequence);
		}
	});

	it('should have pipe key that passes through as literal', () => {
		const pipeKey = terminalToolbar.find(k => k.value === '|');
		expect(pipeKey).toBeDefined();
		expect(getTerminalSequence('|')).toBe('|');
	});
});
