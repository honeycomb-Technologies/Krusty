import { writable } from 'svelte/store';

interface KeyboardState {
	active: boolean;
	height: number;
}

const initial: KeyboardState = { active: false, height: 0 };

export const keyboardStore = writable<KeyboardState>(initial);

function syncCssVariables(state: KeyboardState) {
	if (typeof document === 'undefined') return;
	document.documentElement.style.setProperty('--keyboard-height', `${state.height}px`);
	if (state.active) {
		document.documentElement.setAttribute('data-keyboard-active', '');
	} else {
		document.documentElement.removeAttribute('data-keyboard-active');
	}
}

export function setNativeKeyboardState(active: boolean, height: number) {
	keyboardStore.update(() => {
		const next: KeyboardState = active
			? { active: true, height }
			: { active: false, height: 0 };
		syncCssVariables(next);
		return next;
	});
}
