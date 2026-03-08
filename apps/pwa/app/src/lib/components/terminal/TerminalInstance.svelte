<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { browser } from '$app/environment';
	import { terminalStore, connectTerminal, disconnectTerminal, sendInput, sendResize } from '$stores/terminal';

	interface Props {
		tabId: string;
		isActive: boolean;
	}

	let { tabId, isActive }: Props = $props();

	let terminalContainer: HTMLDivElement;
	let terminal: any;
	let fitAddon: any;
	let webglAddon: any;
	let resizeObserver: ResizeObserver | null = null;
	let resizeTimeout: ReturnType<typeof setTimeout> | null = null;
	let flushFrame: number | null = null;
	let pendingOutput = '';
	let lastCols = 0;
	let lastRows = 0;
	let initialized = false;

	// Get this tab's state
	let tabState = $derived($terminalStore.tabs.find((t) => t.id === tabId));

	onMount(() => {
		if (!browser) return;
		initTerminal();
	});

	function flushOutputQueue() {
		if (!terminal || pendingOutput.length === 0) return;
		terminal.write(pendingOutput);
		pendingOutput = '';
	}

	function scheduleOutputFlush() {
		if (flushFrame !== null) return;
		flushFrame = requestAnimationFrame(() => {
			flushFrame = null;
			flushOutputQueue();
		});
	}

	function queueOutput(data: string) {
		pendingOutput += data;
		scheduleOutputFlush();
	}

	function fitAndResize(force = false) {
		if (!terminal || !fitAddon) return;
		fitAddon.fit();
		const cols = terminal.cols;
		const rows = terminal.rows;
		if (!force && cols === lastCols && rows === lastRows) return;
		lastCols = cols;
		lastRows = rows;
		sendResize(tabId, cols, rows);
	}

	async function initTerminal() {
		const { Terminal } = await import('@xterm/xterm');
		const { FitAddon } = await import('@xterm/addon-fit');
		const { WebLinksAddon } = await import('@xterm/addon-web-links');
		let WebglAddonCtor: any = null;
		try {
			const { WebglAddon } = await import('@xterm/addon-webgl');
			WebglAddonCtor = WebglAddon;
		} catch {
			WebglAddonCtor = null;
		}

		await import('@xterm/xterm/css/xterm.css');

		terminal = new Terminal({
			cursorBlink: true,
			cursorStyle: 'block',
			fontSize: 14,
			fontFamily: 'JetBrains Mono, Fira Code, monospace',
			theme: {
				background: '#0a0a0a',
				foreground: '#fafafa',
				cursor: '#fafafa',
				cursorAccent: '#0a0a0a',
				selectionBackground: '#3f3f46',
				black: '#18181b',
				red: '#ef4444',
				green: '#22c55e',
				yellow: '#eab308',
				blue: '#3b82f6',
				magenta: '#a855f7',
				cyan: '#06b6d4',
				white: '#fafafa',
				brightBlack: '#52525b',
				brightRed: '#f87171',
				brightGreen: '#4ade80',
				brightYellow: '#facc15',
				brightBlue: '#60a5fa',
				brightMagenta: '#c084fc',
				brightCyan: '#22d3ee',
				brightWhite: '#ffffff'
			}
		});

		fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);
		terminal.loadAddon(new WebLinksAddon());
		if (WebglAddonCtor) {
			try {
				webglAddon = new WebglAddonCtor();
				webglAddon.onContextLoss?.(() => {
					try {
						webglAddon?.dispose();
					} catch {
						// Ignore context-loss disposal failures and stay on non-WebGL rendering.
					}
					webglAddon = null;
				});
				terminal.loadAddon(webglAddon);
			} catch {
				webglAddon = null;
			}
		}

		terminal.open(terminalContainer);

		requestAnimationFrame(() => fitAndResize(true));

		terminal.onData((data: string) => {
			sendInput(tabId, data);
		});

		resizeObserver = new ResizeObserver(() => {
			if (!isActive) return;
			if (resizeTimeout) {
				clearTimeout(resizeTimeout);
			}
			resizeTimeout = setTimeout(() => fitAndResize(), 75);
		});
		resizeObserver.observe(terminalContainer);

		connectTerminal(tabId, (data: string) => {
			queueOutput(data);
		});

		const unsubscribe = terminalStore.subscribe((state) => {
			const tab = state.tabs.find((t) => t.id === tabId);
			if (tab?.connected && terminal && !initialized) {
				initialized = true;
				setTimeout(() => fitAndResize(true), 100);
				unsubscribe();
			}
		});
	}

	// Re-fit when becoming active
	$effect(() => {
		if (isActive && terminal && fitAddon) {
			requestAnimationFrame(() => {
				flushOutputQueue();
				fitAndResize(true);
			});
		}
	});

	onDestroy(() => {
		if (resizeTimeout) {
			clearTimeout(resizeTimeout);
			resizeTimeout = null;
		}
		if (flushFrame !== null) {
			cancelAnimationFrame(flushFrame);
			flushFrame = null;
		}
		resizeObserver?.disconnect();
		pendingOutput = '';
		disconnectTerminal(tabId);
		terminal?.dispose();
		webglAddon = null;
	});

	export function focus() {
		terminal?.focus();
	}

	function handleTerminalTap() {
		terminal?.focus();
	}
</script>

<div
	class="terminal-instance"
	class:hidden={!isActive}
	style:padding-bottom="var(--keyboard-height)"
	role="button"
	tabindex="0"
	onclick={handleTerminalTap}
	onkeydown={(e) => e.key === 'Enter' && handleTerminalTap()}
>
	<div bind:this={terminalContainer} class="terminal-container"></div>

	{#if tabState && !tabState.connected}
		<div class="connection-overlay">
			<div class="connection-status">
				{#if tabState.error}
					<div class="error-icon">!</div>
					<p class="error-text">{tabState.error}</p>
						<button
							onclick={() => connectTerminal(tabId, (data) => queueOutput(data))}
							class="reconnect-btn"
						>
						Reconnect
					</button>
				{:else}
					<div class="connecting-spinner"></div>
					<p class="connecting-text">Connecting...</p>
				{/if}
			</div>
		</div>
	{/if}

</div>

<style>
	.terminal-instance {
		position: relative;
		height: 100%;
		background: #0a0a0a;
	}

	.terminal-instance:focus {
		outline: none;
	}

	.terminal-instance:focus-visible {
		outline: 2px solid rgba(255, 107, 53, 0.5);
		outline-offset: -2px;
	}

	.terminal-instance.hidden {
		display: none;
	}

	.terminal-container {
		height: 100%;
		padding: 0.5rem;
	}

	.terminal-container :global(.xterm) {
		height: 100%;
	}

	.connection-overlay {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
		background: hsl(var(--background) / 0.9);
		backdrop-filter: blur(4px);
	}

	.connection-status {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1rem;
		padding: 2rem;
		border-radius: 1rem;
		background: hsl(var(--card));
		border: 1px solid hsl(var(--border) / 0.5);
	}

	.error-icon {
		width: 3rem;
		height: 3rem;
		display: flex;
		align-items: center;
		justify-content: center;
		border-radius: 50%;
		background: hsl(var(--destructive) / 0.2);
		color: hsl(var(--destructive));
		font-size: 1.5rem;
		font-weight: bold;
	}

	.error-text {
		color: hsl(var(--destructive));
		font-size: 0.875rem;
	}

	.reconnect-btn {
		padding: 0.625rem 1.25rem;
		border-radius: 0.5rem;
		font-size: 0.875rem;
		font-weight: 500;
		background: linear-gradient(135deg, #ff6b35, #e85d04);
		color: white;
		border: none;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.reconnect-btn:hover {
		transform: translateY(-1px);
	}

	.connecting-spinner {
		width: 2rem;
		height: 2rem;
		border: 3px solid hsl(var(--muted));
		border-top-color: #ff6b35;
		border-radius: 50%;
		animation: spin 1s linear infinite;
	}

	.connecting-text {
		color: hsl(var(--muted-foreground));
		font-size: 0.875rem;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}
</style>
