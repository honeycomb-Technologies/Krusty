<script lang="ts">
	interface Props {
		onAction: (command: string) => void;
	}

	let { onAction }: Props = $props();

	interface QuickAction {
		label: string;
		command: string;
		type: 'signal' | 'nav' | 'text' | 'enter';
		tooltip: string;
	}

	// Signals = instant action, Text = just types it, Enter = submit
	const quickActions: QuickAction[] = [
		{ label: '⌃C', command: '\x03', type: 'signal', tooltip: 'Cancel' },
		{ label: '⌃D', command: '\x04', type: 'signal', tooltip: 'Exit/EOF' },
		{ label: '↑', command: '\x1b[A', type: 'nav', tooltip: 'History up' },
		{ label: '↓', command: '\x1b[B', type: 'nav', tooltip: 'History down' },
		{ label: 'Tab', command: '\t', type: 'nav', tooltip: 'Complete' },
		{ label: 'ls', command: 'ls -la', type: 'text', tooltip: 'Type: ls -la' },
		{ label: 'cd', command: 'cd ', type: 'text', tooltip: 'Type: cd' },
		{ label: 'clear', command: 'clear', type: 'text', tooltip: 'Type: clear' },
		{ label: '⏎', command: '\r', type: 'enter', tooltip: 'Enter / Run' }
	];
</script>

<div class="quick-actions safe-bottom">
	{#each quickActions as action}
		<button
			onclick={() => onAction(action.command)}
			class="quick-btn {action.type}"
			title={action.tooltip}
		>
			{action.label}
		</button>
	{/each}
</div>

<style>
	.quick-actions {
		display: flex;
		align-items: center;
		gap: 0.375rem;
		overflow-x: auto;
		overscroll-behavior-x: contain;
		padding: 0.5rem 0.5rem;
		border-top: 1px solid hsl(var(--border) / 0.5);
		background: hsl(var(--card) / 0.95);
		backdrop-filter: blur(8px);
		-webkit-backdrop-filter: blur(8px);

		/* Hide scrollbar on mobile */
		scrollbar-width: none;
		-ms-overflow-style: none;
	}

	.quick-actions::-webkit-scrollbar {
		display: none;
	}

	.quick-btn {
		flex-shrink: 0;
		padding: 0.375rem 0.625rem;
		border-radius: 0.375rem;
		font-size: 0.6875rem;
		font-weight: 500;
		font-family: ui-monospace, SFMono-Regular, 'SF Mono', monospace;
		background: hsl(var(--muted));
		color: hsl(var(--muted-foreground));
		border: 1px solid hsl(var(--border) / 0.5);
		transition: all 0.15s ease;
		touch-action: manipulation;
		-webkit-tap-highlight-color: transparent;
	}

	.quick-btn:hover {
		background: hsl(var(--accent));
		color: hsl(var(--accent-foreground));
	}

	.quick-btn:active {
		transform: scale(0.95);
	}

	/* Signal buttons (Ctrl+C, etc) - rusty/warning color */
	.quick-btn.signal {
		background: hsl(25 95% 20% / 0.6);
		color: #ff6b35;
		border-color: hsl(25 95% 35% / 0.5);
	}

	.quick-btn.signal:hover {
		background: hsl(25 95% 25% / 0.8);
	}

	/* Navigation buttons (arrows, tab) - subtle */
	.quick-btn.nav {
		background: hsl(var(--muted));
		color: hsl(var(--muted-foreground));
		min-width: 2rem;
	}

	.quick-btn.nav:hover {
		background: hsl(var(--accent));
		color: hsl(var(--accent-foreground));
	}

	/* Text buttons (type commands) - primary accent */
	.quick-btn.text {
		background: hsl(var(--primary) / 0.15);
		color: hsl(var(--primary));
		border-color: hsl(var(--primary) / 0.3);
	}

	.quick-btn.text:hover {
		background: hsl(var(--primary) / 0.25);
	}

	/* Enter button - prominent green */
	.quick-btn.enter {
		background: hsl(142 71% 25% / 0.8);
		color: hsl(142 71% 70%);
		border-color: hsl(142 71% 35% / 0.5);
		font-size: 0.875rem;
		min-width: 2.25rem;
	}

	.quick-btn.enter:hover {
		background: hsl(142 71% 30% / 0.9);
	}
</style>
