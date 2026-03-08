/**
 * Keyboard layout definitions for VirtualKeyboard
 * Supports QWERTY, numbers, and symbols modes
 * 
 * This module re-exports all layout types and utilities for convenience.
 * For more granular imports, use the submodules:
 * - ./layouts/types - Types and KeyAction enum
 * - ./layouts/data - QWERTY, numbers, symbols layouts
 * - ./layouts/terminal - Terminal layout and escape sequences
 * - ./layouts/utils - Helper functions
 */

// Re-export types
export { type KeyboardLayout, type KeyConfig, KeyAction } from './layouts/types';

// Re-export layout data
export { qwertyLayout, numbersLayout, symbolsLayout } from './layouts/data';

// Re-export terminal toolbar and sequences
export { terminalToolbar, getTerminalSequence } from './layouts/terminal';

// Re-export utilities
export { getLayout, formatKeyDisplay } from './layouts/utils';
