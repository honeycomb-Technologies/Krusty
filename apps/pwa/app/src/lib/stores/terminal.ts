import { writable, get } from 'svelte/store';
import { browser } from '$app/environment';

export interface TerminalTab {
	id: string;
	title: string;
	connected: boolean;
	error: string | null;
}

interface TerminalState {
	tabs: TerminalTab[];
	activeTabId: string | null;
}

const initialState: TerminalState = {
	tabs: [],
	activeTabId: null
};

const MAX_RECONNECT_ATTEMPTS = 8;
const RECONNECT_INITIAL_DELAY_MS = 250;
const RECONNECT_MAX_DELAY_MS = 5_000;
const RECONNECT_STABLE_RESET_MS = 30_000;
const HEARTBEAT_INTERVAL_MS = 15_000;
const HEARTBEAT_TIMEOUT_MS = 45_000;

export const terminalStore = writable<TerminalState>(initialState);

// Subscribe to workspace changes - cd to directory when it changes
// Imported dynamically to avoid circular dependency
let workspaceSubscribed = false;
let pendingInitialCd: string | null = null;
let workspaceUnsubscribe: (() => void) | null = null;
let workspaceSyncTimeout: ReturnType<typeof setTimeout> | null = null;

const WORKSPACE_SYNC_TIMEOUT = 30_000; // 30 seconds

export function initWorkspaceSync() {
	if (!browser || workspaceSubscribed) return;
	workspaceSubscribed = true;

	import('./workspace').then(({ workspaceStore }) => {
		let lastDir: string | null = null;
		let hasConnected = false;
		workspaceUnsubscribe = workspaceStore.subscribe((ws) => {
			if (ws.initialized && ws.directory && ws.directory !== lastDir) {
				lastDir = ws.directory;
				// cd to new directory in active terminal
				const state = get(terminalStore);
				if (state.activeTabId) {
					const tab = state.tabs.find(t => t.id === state.activeTabId);
					if (tab?.connected) {
						hasConnected = true;
						sendInput(state.activeTabId, `cd "${ws.directory}"\n`);
					} else {
						// Terminal not connected yet, queue the cd
						pendingInitialCd = ws.directory;
					}
				} else {
					// No active tab yet, queue the cd
					pendingInitialCd = ws.directory;
				}
			}
		});

		// Auto-unsubscribe after timeout if no terminal ever connected
		workspaceSyncTimeout = setTimeout(() => {
			if (!hasConnected) {
				cleanupWorkspaceSync();
			}
			workspaceSyncTimeout = null;
		}, WORKSPACE_SYNC_TIMEOUT);
	});
}

export function cleanupWorkspaceSync() {
	if (workspaceSyncTimeout) {
		clearTimeout(workspaceSyncTimeout);
		workspaceSyncTimeout = null;
	}
	workspaceUnsubscribe?.();
	workspaceUnsubscribe = null;
	workspaceSubscribed = false;
}

// Called when terminal connects - execute any pending cd
export function executePendingCd(tabId: string) {
	if (pendingInitialCd) {
		// Small delay to let terminal initialize
		setTimeout(() => {
			sendInput(tabId, `cd "${pendingInitialCd}"\n`);
			pendingInitialCd = null;
		}, 100);
	}
}

// Per-tab WebSocket connections and callbacks
const connections = new Map<string, WebSocket>();
const callbacks = new Map<string, (data: string) => void>();
const manualDisconnects = new Set<string>();
const reconnectAttempts = new Map<string, number>();
const reconnectTimers = new Map<string, ReturnType<typeof setTimeout>>();
const stableResetTimers = new Map<string, ReturnType<typeof setTimeout>>();
const heartbeatIntervals = new Map<string, ReturnType<typeof setInterval>>();
const heartbeatTimeouts = new Map<string, ReturnType<typeof setTimeout>>();
const textDecoder = new TextDecoder();

let tabCounter = 0;

function generateTabId(): string {
	return `term-${++tabCounter}`;
}

function tabExists(tabId: string): boolean {
	return get(terminalStore).tabs.some((tab) => tab.id === tabId);
}

function updateTab(tabId: string, updater: (tab: TerminalTab) => TerminalTab) {
	terminalStore.update((s) => ({
		...s,
		tabs: s.tabs.map((tab) => (tab.id === tabId ? updater(tab) : tab))
	}));
}

function clearReconnectTimer(tabId: string) {
	const timer = reconnectTimers.get(tabId);
	if (timer) {
		clearTimeout(timer);
		reconnectTimers.delete(tabId);
	}
}

function clearStableResetTimer(tabId: string) {
	const timer = stableResetTimers.get(tabId);
	if (timer) {
		clearTimeout(timer);
		stableResetTimers.delete(tabId);
	}
}

function clearHeartbeat(tabId: string) {
	const interval = heartbeatIntervals.get(tabId);
	if (interval) {
		clearInterval(interval);
		heartbeatIntervals.delete(tabId);
	}

	const timeout = heartbeatTimeouts.get(tabId);
	if (timeout) {
		clearTimeout(timeout);
		heartbeatTimeouts.delete(tabId);
	}
}

function touchHeartbeat(tabId: string, ws: WebSocket) {
	const timeout = heartbeatTimeouts.get(tabId);
	if (timeout) {
		clearTimeout(timeout);
	}

	const nextTimeout = setTimeout(() => {
		if (connections.get(tabId) === ws && ws.readyState === WebSocket.OPEN) {
			ws.close(4000, 'heartbeat_timeout');
		}
	}, HEARTBEAT_TIMEOUT_MS);
	heartbeatTimeouts.set(tabId, nextTimeout);
}

function startHeartbeat(tabId: string, ws: WebSocket) {
	clearHeartbeat(tabId);
	touchHeartbeat(tabId, ws);

	const interval = setInterval(() => {
		if (connections.get(tabId) !== ws || ws.readyState !== WebSocket.OPEN) {
			return;
		}

		try {
			ws.send(JSON.stringify({ type: 'ping' }));
		} catch {
			ws.close();
		}
	}, HEARTBEAT_INTERVAL_MS);

	heartbeatIntervals.set(tabId, interval);
}

function scheduleStableReset(tabId: string, ws: WebSocket) {
	clearStableResetTimer(tabId);
	const timer = setTimeout(() => {
		if (connections.get(tabId) === ws && ws.readyState === WebSocket.OPEN) {
			reconnectAttempts.delete(tabId);
		}
		stableResetTimers.delete(tabId);
	}, RECONNECT_STABLE_RESET_MS);
	stableResetTimers.set(tabId, timer);
}

function scheduleReconnect(tabId: string) {
	if (!tabExists(tabId) || manualDisconnects.has(tabId) || reconnectTimers.has(tabId)) {
		return;
	}

	const attempt = reconnectAttempts.get(tabId) ?? 0;
	if (attempt >= MAX_RECONNECT_ATTEMPTS) {
		updateTab(tabId, (tab) => ({
			...tab,
			error: 'Connection lost. Retry limit reached.'
		}));
		return;
	}

	const attemptNumber = attempt + 1;
	const delay = Math.min(
		RECONNECT_INITIAL_DELAY_MS * 2 ** attempt,
		RECONNECT_MAX_DELAY_MS
	);
	reconnectAttempts.set(tabId, attemptNumber);

	updateTab(tabId, (tab) => ({
		...tab,
		error: `Reconnecting (${attemptNumber}/${MAX_RECONNECT_ATTEMPTS})...`
	}));

	const timer = setTimeout(() => {
		reconnectTimers.delete(tabId);
		if (!tabExists(tabId) || manualDisconnects.has(tabId)) {
			return;
		}

		const callback = callbacks.get(tabId);
		if (!callback) {
			return;
		}

		connectTerminalInternal(tabId, callback, true);
	}, delay);

	reconnectTimers.set(tabId, timer);
}

export function createTab(title?: string): string {
	const id = generateTabId();
	const tab: TerminalTab = {
		id,
		title: title || `Terminal ${tabCounter}`,
		connected: false,
		error: null
	};

	terminalStore.update((s) => ({
		tabs: [...s.tabs, tab],
		activeTabId: id
	}));

	return id;
}

export function closeTab(tabId: string) {
	disconnectTerminal(tabId);

	terminalStore.update((s) => {
		const newTabs = s.tabs.filter((t) => t.id !== tabId);
		let newActiveId = s.activeTabId;

		// If closing active tab, switch to another
		if (s.activeTabId === tabId) {
			const closedIndex = s.tabs.findIndex((t) => t.id === tabId);
				if (newTabs.length > 0) {
					// Prefer tab to the left, otherwise first tab
					newActiveId = newTabs[Math.max(0, closedIndex - 1)]?.id || newTabs[0]?.id;
				} else {
					newActiveId = null;
				}
			}

			if (newTabs.length === 0) {
				cleanupWorkspaceSync();
			}

			return { tabs: newTabs, activeTabId: newActiveId };
		});
}

export function setActiveTab(tabId: string) {
	terminalStore.update((s) => ({ ...s, activeTabId: tabId }));
}

export function renameTab(tabId: string, title: string) {
	terminalStore.update((s) => ({
		...s,
		tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, title } : t))
	}));
}

function connectTerminalInternal(
	tabId: string,
	onData: (data: string) => void,
	isReconnect: boolean
) {
	if (!browser) return;

	// Initialize workspace sync on first connection
	initWorkspaceSync();
	manualDisconnects.delete(tabId);
	clearReconnectTimer(tabId);

	// Already connected?
	const existingWs = connections.get(tabId);
	if (
		existingWs?.readyState === WebSocket.OPEN ||
		existingWs?.readyState === WebSocket.CONNECTING
	) {
		callbacks.set(tabId, onData);
		return;
	}

	if (existingWs && existingWs.readyState !== WebSocket.CLOSED) {
		existingWs.close();
	}
	clearHeartbeat(tabId);
	connections.delete(tabId);

	callbacks.set(tabId, onData);
	if (!isReconnect) {
		reconnectAttempts.delete(tabId);
		updateTab(tabId, (tab) => ({ ...tab, connected: false, error: null }));
	}

	const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
	const wsUrl = `${protocol}//${window.location.host}/ws/terminal`;

	const ws = new WebSocket(wsUrl);
	ws.binaryType = 'arraybuffer';
	connections.set(tabId, ws);

	ws.onopen = () => {
		updateTab(tabId, (tab) => ({ ...tab, connected: true, error: null }));
		scheduleStableReset(tabId, ws);
		startHeartbeat(tabId, ws);
		try {
			ws.send(JSON.stringify({ type: 'hello', binary_output: true }));
		} catch {
			ws.close();
			return;
		}

		// Execute any pending cd from workspace initialization
		executePendingCd(tabId);
	};

	ws.onmessage = (event) => {
		const callback = callbacks.get(tabId);
		if (!callback) return;
		touchHeartbeat(tabId, ws);

		if (event.data instanceof ArrayBuffer) {
			callback(textDecoder.decode(new Uint8Array(event.data)));
			return;
		}

		if (event.data instanceof Blob) {
			void event.data.arrayBuffer().then((buffer) => {
				callback(textDecoder.decode(new Uint8Array(buffer)));
			});
			return;
		}

		const raw = typeof event.data === 'string' ? event.data : String(event.data);

		try {
			const msg = JSON.parse(raw);
			if (msg.type === 'output' && typeof msg.data === 'string') {
				callback(msg.data);
			} else if (msg.type === 'error' && typeof msg.error === 'string') {
				updateTab(tabId, (tab) => ({ ...tab, error: msg.error }));
			}
		} catch {
			callback(raw);
		}
	};

	ws.onerror = () => {
		updateTab(tabId, (tab) => ({
			...tab,
			error: tab.error || 'WebSocket error'
		}));
	};

	ws.onclose = () => {
		clearHeartbeat(tabId);
		clearStableResetTimer(tabId);
		connections.delete(tabId);
		updateTab(tabId, (tab) => ({ ...tab, connected: false }));

		if (manualDisconnects.has(tabId)) {
			manualDisconnects.delete(tabId);
			return;
		}

		scheduleReconnect(tabId);
	};
}

export function connectTerminal(tabId: string, onData: (data: string) => void) {
	connectTerminalInternal(tabId, onData, false);
}

export function disconnectTerminal(tabId: string) {
	manualDisconnects.add(tabId);
	clearReconnectTimer(tabId);
	clearStableResetTimer(tabId);
	clearHeartbeat(tabId);
	reconnectAttempts.delete(tabId);

	const ws = connections.get(tabId);
	ws?.close();
	connections.delete(tabId);
	callbacks.delete(tabId);
}

export function sendInput(tabId: string, data: string) {
	const ws = connections.get(tabId);
	if (ws?.readyState === WebSocket.OPEN) {
		ws.send(JSON.stringify({ type: 'input', data }));
	}
}

export function sendResize(tabId: string, cols: number, rows: number) {
	const ws = connections.get(tabId);
	if (ws?.readyState === WebSocket.OPEN) {
		ws.send(JSON.stringify({ type: 'resize', cols, rows }));
	}
}

// Helper to get current tab's connection state
export function getTabState(tabId: string): TerminalTab | undefined {
	return get(terminalStore).tabs.find((t) => t.id === tabId);
}
