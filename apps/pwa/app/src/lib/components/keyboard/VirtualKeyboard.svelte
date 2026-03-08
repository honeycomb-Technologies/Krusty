<script lang="ts">
	import type { KeyConfig, KeyboardLayout } from './layouts';
	import { getLayout, getTerminalSequence, formatKeyDisplay, KeyAction, terminalToolbar } from './layouts';

	interface Props {
		mode?: 'chat' | 'terminal';
		visible?: boolean;
		onKeyPress?: (key: string, isEnter: boolean) => void;
		onClose?: () => void;
		onHeightChange?: (height: number) => void;
	}

	let {
		mode = 'chat',
		visible = true,
		onKeyPress,
		onClose,
		onHeightChange
	}: Props = $props();

	let isTerminalMode = $derived(mode === 'terminal');

	let currentLayout = $state<KeyboardLayout>('qwerty');
	let isShifted = $state(false);
	let ctrlActive = $state(false);
	let keyboardEl: HTMLDivElement | undefined = $state(undefined);

	let layout = $derived(getLayout(currentLayout));

	// Report keyboard height to parent when visible
	$effect(() => {
		if (visible && keyboardEl) {
			const height = keyboardEl.offsetHeight;
			onHeightChange?.(height);
		} else if (!visible) {
			onHeightChange?.(0);
		}
	});

	// Track whether a touch event already handled the key press
	let touchHandled = false;

	// Key repeat state — initial delay then fast repeat (like native keyboards)
	let repeatTimeout: ReturnType<typeof setTimeout> | null = null;
	let repeatInterval: ReturnType<typeof setInterval> | null = null;
	const REPEAT_DELAY_MS = 400;
	const REPEAT_INTERVAL_MS = 80;

	function startRepeat(key: KeyConfig) {
		handleKeyAction(key);
		repeatTimeout = setTimeout(() => {
			repeatInterval = setInterval(() => {
				handleKeyAction(key);
			}, REPEAT_INTERVAL_MS);
		}, REPEAT_DELAY_MS);
	}

	function stopRepeat() {
		if (repeatTimeout) {
			clearTimeout(repeatTimeout);
			repeatTimeout = null;
		}
		if (repeatInterval) {
			clearInterval(repeatInterval);
			repeatInterval = null;
		}
	}

	// Key preview popup state
	let previewKey: KeyConfig | null = $state(null);
	let previewPosition = $state({ x: 0, y: 0 });
	let previewTimeout: ReturnType<typeof setTimeout> | null = null;

	function showPreview(key: KeyConfig, event: MouseEvent | TouchEvent) {
		if (key.action) return;
		const clientX = 'touches' in event ? event.touches[0].clientX : (event as MouseEvent).clientX;
		const clientY = 'touches' in event ? event.touches[0].clientY : (event as MouseEvent).clientY;
		previewPosition = { x: clientX, y: clientY - 60 };
		previewKey = key;
		if (previewTimeout) clearTimeout(previewTimeout);
	}

	function hidePreview() {
		if (previewTimeout) {
			clearTimeout(previewTimeout);
			previewTimeout = null;
		}
		previewKey = null;
	}

	function handleKeyAction(key: KeyConfig) {
		if (navigator.vibrate) navigator.vibrate(10);

		if (isTerminalMode) {
			handleTerminalKeyPress(key);
			return;
		}
		handleChatKeyPress(key);
	}

	function handleKeyPress(key: KeyConfig) {
		handleKeyAction(key);
	}

	function handleToolbarKeyPress(key: KeyConfig) {
		if (navigator.vibrate) navigator.vibrate(10);
		const value = key.value;

		if (value === 'ctrl') {
			ctrlActive = !ctrlActive;
			return;
		}

		// If ctrl is active, send ctrl+key for single chars
		if (ctrlActive && value.length === 1) {
			const code = value.toUpperCase().charCodeAt(0) - 64;
			if (code >= 1 && code <= 26) {
				onKeyPress?.(String.fromCharCode(code), false);
			}
			ctrlActive = false;
			return;
		}

		const sequence = getTerminalSequence(value);
		if (sequence !== value) {
			onKeyPress?.(sequence, false);
		} else {
			onKeyPress?.(value, false);
		}
		ctrlActive = false;
	}

	function handleTerminalKeyPress(key: KeyConfig) {
		const action = key.action;
		const value = key.value;

		switch (action) {
			case KeyAction.Shift:
				isShifted = !isShifted;
				return;
			case KeyAction.Backspace:
				onKeyPress?.('\x7f', false);
				return;
			case KeyAction.Space:
				onKeyPress?.(' ', false);
				return;
			case KeyAction.Enter:
				onKeyPress?.('\n', true);
				return;
			case KeyAction.SwitchNumbers:
				currentLayout = 'numbers';
				return;
			case KeyAction.SwitchSymbols:
				currentLayout = 'symbols';
				return;
			case KeyAction.SwitchQwerty:
				currentLayout = 'qwerty';
				return;
		}

		// If ctrl is active, send control character
		if (ctrlActive && value.length === 1 && value.match(/[a-z]/i)) {
			const code = value.toUpperCase().charCodeAt(0) - 64;
			if (code >= 1 && code <= 26) {
				onKeyPress?.(String.fromCharCode(code), false);
			}
			ctrlActive = false;
			return;
		}

		const sequence = getTerminalSequence(value);
		if (sequence !== value) {
			onKeyPress?.(sequence, false);
		} else {
			let char = value;
			if (isShifted && char.length === 1 && char.match(/[a-z]/)) {
				char = char.toUpperCase();
			}
			onKeyPress?.(char, false);
			if (isShifted && currentLayout === 'qwerty') {
				isShifted = false;
			}
		}
		ctrlActive = false;
	}

	function handleChatKeyPress(key: KeyConfig) {
		const action = key.action;

		switch (action) {
			case KeyAction.Shift:
				isShifted = !isShifted;
				break;
			case KeyAction.Backspace:
				onKeyPress?.('\x7f', false);
				break;
			case KeyAction.Space:
				onKeyPress?.(' ', false);
				break;
			case KeyAction.Enter:
				onKeyPress?.('\n', true);
				break;
			case KeyAction.SwitchNumbers:
				currentLayout = 'numbers';
				isShifted = false;
				break;
			case KeyAction.SwitchSymbols:
				currentLayout = 'symbols';
				isShifted = false;
				break;
			case KeyAction.SwitchQwerty:
				currentLayout = 'qwerty';
				isShifted = false;
				break;
			default: {
				let char = key.value;
				if (isShifted && char.length === 1 && char.match(/[a-z]/)) {
					char = char.toUpperCase();
				}
				onKeyPress?.(char, false);
				if (isShifted && currentLayout === 'qwerty') {
					isShifted = false;
				}
				break;
			}
		}
	}

	// Swipe down dismissal
	let touchStartY = $state(0);
	let touchStartTime = $state(0);
	const SWIPE_THRESHOLD = 100;
	const SWIPE_TIME_LIMIT = 500;

	function handleTouchStart(e: TouchEvent) {
		touchStartY = e.touches[0].clientY;
		touchStartTime = Date.now();
	}

	function handleTouchEnd(e: TouchEvent) {
		const deltaY = e.changedTouches[0].clientY - touchStartY;
		const elapsed = Date.now() - touchStartTime;
		if (deltaY > SWIPE_THRESHOLD && elapsed < SWIPE_TIME_LIMIT) {
			onClose?.();
		}
	}

	function handleBackdropClick(e: MouseEvent) {
		const target = e.target as HTMLElement;
		if (target.classList.contains('keyboard-backdrop')) {
			onClose?.();
		}
	}
</script>

{#if visible}
	<div
		class="keyboard-backdrop"
		onclick={handleBackdropClick}
		role="button"
		tabindex="-1"
		aria-hidden="true"
	></div>

	{#if previewKey}
		<div
			class="key-preview"
			style="left: {previewPosition.x}px; top: {previewPosition.y}px;"
			aria-hidden="true"
		>
			{isShifted && previewKey.value.length === 1 && previewKey.value.match(/[a-z]/)
				? previewKey.value.toUpperCase()
				: previewKey.display || previewKey.value}
		</div>
	{/if}

	<div
		bind:this={keyboardEl}
		class="keyboard-container"
		role="group"
		aria-label="Virtual keyboard"
		ontouchstart={handleTouchStart}
		ontouchend={handleTouchEnd}
	>
		{#if isTerminalMode}
			<div class="toolbar-row">
				{#each terminalToolbar as key}
					<button
						class="toolbar-key"
						class:ctrl-active={key.value === 'ctrl' && ctrlActive}
						onclick={() => {
							if (touchHandled) { touchHandled = false; return; }
							handleToolbarKeyPress(key);
						}}
						ontouchstart={(e) => {
							e.preventDefault();
							touchHandled = true;
							handleToolbarKeyPress(key);
						}}
						ontouchend={() => {}}
						aria-label={key.display || key.value}
						type="button"
					>
						<span class="key-content">{key.display || key.value}</span>
					</button>
				{/each}
			</div>
		{/if}

		{#each layout as row}
			<div class="keyboard-row">
				{#each row as key}
					<button
						class="key"
						class:shift-active={key.action === KeyAction.Shift && isShifted}
						data-width={key.width}
						data-action={key.action}
						onclick={() => {
							if (touchHandled) { touchHandled = false; return; }
							handleKeyPress(key);
						}}
						onmousedown={(e) => {
							if (key.action === KeyAction.Backspace) startRepeat(key);
							else showPreview(key, e);
						}}
						onmouseup={() => { stopRepeat(); hidePreview(); }}
						onmouseleave={() => { stopRepeat(); hidePreview(); }}
						ontouchstart={(e) => {
							e.preventDefault();
							touchHandled = true;
							if (key.action === KeyAction.Backspace) startRepeat(key);
							else handleKeyPress(key);
						}}
						ontouchend={() => { stopRepeat(); hidePreview(); }}
						aria-label={key.display || key.value}
						aria-pressed={key.action === KeyAction.Shift ? isShifted : undefined}
						type="button"
					>
						<span class="key-content">
							{formatKeyDisplay(key.value, key.display)}
						</span>
					</button>
				{/each}
			</div>
		{/each}
	</div>
{/if}

<style>
	.keyboard-backdrop {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		bottom: 0;
		z-index: 999;
	}

	.key-preview {
		position: fixed;
		transform: translateX(-50%) translateY(-100%);
		z-index: 1001;
		min-width: 50px;
		height: 50px;
		display: flex;
		align-items: center;
		justify-content: center;
		background: rgba(40, 40, 50, 0.95);
		backdrop-filter: blur(12px);
		-webkit-backdrop-filter: blur(12px);
		border-radius: 12px;
		box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
		color: white;
		font-size: 24px;
		font-weight: 500;
		font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
		animation: previewPop 0.15s ease-out;
		pointer-events: none;
	}

	@keyframes previewPop {
		from {
			transform: translateX(-50%) translateY(-100%) scale(0.8);
			opacity: 0;
		}
		to {
			transform: translateX(-50%) translateY(-100%) scale(1);
			opacity: 1;
		}
	}

	.keyboard-container {
		position: fixed;
		bottom: calc(4rem + var(--safe-area-bottom));
		left: 0;
		right: 0;
		z-index: 1000;
		background: rgba(20, 20, 25, 0.85);
		backdrop-filter: blur(24px);
		-webkit-backdrop-filter: blur(24px);
		border-top: 1px solid rgba(255, 255, 255, 0.08);
		border-radius: 20px 20px 0 0;
		padding: 8px 4px;
		animation: slideUp 0.25s cubic-bezier(0.16, 1, 0.3, 1);
		user-select: none;
		-webkit-user-select: none;
		touch-action: manipulation;
	}

	@keyframes slideUp {
		from {
			transform: translateY(100%);
			opacity: 0;
		}
		to {
			transform: translateY(0);
			opacity: 1;
		}
	}

	/* Terminal toolbar row — sits above the main keyboard */
	.toolbar-row {
		display: flex;
		gap: 4px;
		padding: 0 4px 6px;
		margin-bottom: 6px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
	}

	.toolbar-key {
		display: flex;
		align-items: center;
		justify-content: center;
		flex: 1;
		height: 36px;
		padding: 0;
		border: none;
		border-radius: 6px;
		background: rgba(255, 255, 255, 0.1);
		color: rgba(255, 255, 255, 0.85);
		font-size: 13px;
		font-weight: 500;
		font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
		transition: all 0.1s ease;
		cursor: pointer;
		-webkit-tap-highlight-color: transparent;
	}

	.toolbar-key:active {
		background: rgba(255, 255, 255, 0.2);
		transform: scale(0.95);
	}

	.toolbar-key.ctrl-active {
		background: rgba(255, 107, 53, 0.3);
		color: #ff6b35;
	}

	.keyboard-row {
		display: flex;
		gap: 4px;
		margin-bottom: 6px;
		padding: 0 4px;
	}

	.keyboard-row:last-of-type {
		margin-bottom: 0;
	}

	.key {
		display: flex;
		align-items: center;
		justify-content: center;
		flex: 1;
		height: 44px;
		padding: 0;
		border: none;
		border-radius: 8px;
		background: rgba(255, 255, 255, 0.06);
		color: rgba(255, 255, 255, 0.9);
		font-size: 16px;
		font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
		font-weight: 400;
		transition: all 0.1s ease;
		cursor: pointer;
		-webkit-tap-highlight-color: transparent;
	}

	.key:active {
		background: rgba(255, 255, 255, 0.15);
		transform: scale(0.96);
	}

	.key[data-width="wide"] {
		flex: 1.5;
		font-size: 14px;
		font-weight: 500;
	}

	.key[data-width="extra-wide"] {
		flex: 4;
	}

	.key[data-action] {
		background: rgba(255, 255, 255, 0.1);
		font-weight: 500;
	}

	.key.shift-active {
		background: rgba(255, 107, 53, 0.3);
		color: #ff6b35;
	}

	.key-content {
		display: flex;
		align-items: center;
		justify-content: center;
		line-height: 1;
	}

	:global(.light) .keyboard-container {
		background: rgba(255, 255, 255, 0.9);
		border-top-color: rgba(0, 0, 0, 0.1);
	}

	:global(.light) .key {
		background: rgba(0, 0, 0, 0.05);
		color: rgba(0, 0, 0, 0.8);
	}

	:global(.light) .key[data-action] {
		background: rgba(0, 0, 0, 0.1);
	}

	:global(.light) .key:active {
		background: rgba(0, 0, 0, 0.15);
	}

	:global(.light) .toolbar-key {
		background: rgba(0, 0, 0, 0.08);
		color: rgba(0, 0, 0, 0.7);
	}

	:global(.light) .toolbar-row {
		border-bottom-color: rgba(0, 0, 0, 0.06);
	}

	@media (orientation: landscape) and (max-height: 400px) {
		.keyboard-container {
			padding: 4px;
		}

		.keyboard-row {
			gap: 3px;
			margin-bottom: 4px;
		}

		.key {
			height: 36px;
			font-size: 14px;
		}

		.key[data-width="wide"] {
			font-size: 12px;
		}

		.toolbar-row {
			padding-bottom: 4px;
			margin-bottom: 4px;
		}

		.toolbar-key {
			height: 28px;
			font-size: 11px;
		}
	}

	@media (min-width: 768px) {
		.key {
			height: 48px;
			font-size: 18px;
		}

		.toolbar-key {
			height: 40px;
			font-size: 14px;
		}
	}
</style>
