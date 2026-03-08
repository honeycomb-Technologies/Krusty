<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import {
		RefreshCw,
		Pin,
		PinOff,
		EyeOff,
		ExternalLink,
		Copy,
		AlertCircle,
		ChevronLeft,
		ChevronRight,
		RotateCw,
		X,
		ArrowRight,
		Plus,
		Ellipsis
	} from 'lucide-svelte';

	import { apiClient, getApiUrl, type PortEntry, type PreviewSettings } from '$api/client';

	const PREVIEW_SESSION_KEY = 'workspace_preview_tabs_v1';

	type PersistedPreviewTab = {
		id: string;
		port: number;
		title: string;
		history: string[];
		historyIndex: number;
	};

	type PreviewTab = PersistedPreviewTab & {
		isLoading: boolean;
		error: string | null;
	};

	let ports = $state<PortEntry[]>([]);
	let settings = $state<PreviewSettings | null>(null);
	let loading = $state(true);
	let refreshing = $state(false);
	let mutatingPort = $state<number | null>(null);
	let error = $state<string | null>(null);
	let discoveryWarning = $state<string | null>(null);
	let copiedPort = $state<number | null>(null);
	let pollTimer: ReturnType<typeof setInterval> | null = null;
	let inFlight = false;
	let previewTabs = $state<PreviewTab[]>([]);
	let activeTabId = $state<string | null>(null);
	let previewAddress = $state('');
	let showNewTab = $state(false);
	let tabMenuOpen = $state<string | null>(null);

	const previewFrames = new Map<string, HTMLIFrameElement>();

	onMount(() => {
		restorePreviewTabs();
		void loadPorts();
	});

	onDestroy(() => {
		clearPollTimer();
		previewFrames.clear();
	});

	function registerPreviewFrame(node: HTMLIFrameElement, tabId: string) {
		previewFrames.set(tabId, node);
		return {
			destroy() {
				previewFrames.delete(tabId);
			}
		};
	}

	function clearPollTimer() {
		if (pollTimer) {
			clearInterval(pollTimer);
			pollTimer = null;
		}
	}

	function resetPollTimer(intervalSecs: number) {
		clearPollTimer();
		const safeInterval = Math.max(2, intervalSecs || 5);
		pollTimer = setInterval(() => {
			void loadPorts(true);
		}, safeInterval * 1000);
	}

	async function loadPorts(background = false) {
		if (inFlight && background) return;
		inFlight = true;
		if (!background) {
			loading = true;
		} else {
			refreshing = true;
		}

		try {
			const response = await apiClient.getPorts();
			ports = response.ports;
			settings = response.settings;
			discoveryWarning = response.discovery_error ?? null;
			error = null;
			resetPollTimer(response.settings.auto_refresh_secs);
			syncTabTitles();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load preview ports';
		} finally {
			inFlight = false;
			loading = false;
			refreshing = false;
		}
	}

	function getPreviewUrl(port: number): string {
		return getApiUrl(`/ports/${port}/proxy`);
	}

	async function copyPreviewUrl(port: number) {
		try {
			await navigator.clipboard.writeText(getPreviewUrl(port));
			copiedPort = port;
			setTimeout(() => {
				if (copiedPort === port) copiedPort = null;
			}, 1500);
		} catch {
			error = 'Failed to copy preview URL';
		}
	}

	function getActiveTab(): PreviewTab | null {
		return previewTabs.find((tab) => tab.id === activeTabId) ?? null;
	}

	function getActiveFrame(): HTMLIFrameElement | null {
		const activeTab = getActiveTab();
		if (!activeTab) return null;
		return previewFrames.get(activeTab.id) ?? null;
	}

	function syncPreviewAddress() {
		const activeTab = getActiveTab();
		previewAddress = activeTab ? activeTab.history[activeTab.historyIndex] ?? '' : '';
	}

	function persistPreviewTabs() {
		if (typeof window === 'undefined') return;
		const payload = {
			tabs: previewTabs.map((tab) => ({
				id: tab.id,
				port: tab.port,
				title: tab.title,
				history: tab.history,
				historyIndex: tab.historyIndex
			})),
			activeTabId
		};
		sessionStorage.setItem(PREVIEW_SESSION_KEY, JSON.stringify(payload));
	}

	function restorePreviewTabs() {
		if (typeof window === 'undefined') return;
		const raw = sessionStorage.getItem(PREVIEW_SESSION_KEY);
		if (!raw) return;

		try {
			const parsed = JSON.parse(raw) as {
				tabs?: PersistedPreviewTab[];
				activeTabId?: string | null;
			};
			const restoredTabs: PreviewTab[] = Array.isArray(parsed.tabs)
				? parsed.tabs
						.filter(
							(tab) =>
								tab &&
								typeof tab.id === 'string' &&
								Number.isInteger(tab.port) &&
								tab.port > 0 &&
								tab.port <= 65535 &&
								Array.isArray(tab.history) &&
								tab.history.length > 0
						)
						.map((tab) => {
							const safeHistory = tab.history
								.map((entry) => normalizeProxyUrl(entry, tab.port))
								.filter((entry): entry is string => Boolean(entry));
							if (safeHistory.length === 0) {
								safeHistory.push(getPreviewUrl(tab.port));
							}
							const boundedIndex = Math.min(
								Math.max(0, Number(tab.historyIndex) || 0),
								safeHistory.length - 1
							);
							return {
								id: tab.id,
								port: tab.port,
								title: tab.title || `Port ${tab.port}`,
								history: safeHistory,
								historyIndex: boundedIndex,
								isLoading: false,
								error: null
							};
						})
				: [];
			previewTabs = restoredTabs;
			if (restoredTabs.length === 0) {
				activeTabId = null;
			} else if (parsed.activeTabId && restoredTabs.some((tab) => tab.id === parsed.activeTabId)) {
				activeTabId = parsed.activeTabId;
			} else {
				activeTabId = restoredTabs[0].id;
			}
			syncPreviewAddress();
		} catch {
			sessionStorage.removeItem(PREVIEW_SESSION_KEY);
		}
	}

	function setPreviewState(nextTabs: PreviewTab[], nextActiveTabId: string | null) {
		previewTabs = nextTabs;
		activeTabId = nextActiveTabId;
		syncPreviewAddress();
		persistPreviewTabs();
	}

	function generateTabId(): string {
		if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
			return crypto.randomUUID();
		}
		return `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
	}

	function findPort(portNumber: number): PortEntry | undefined {
		return ports.find((entry) => entry.port === portNumber);
	}

	function extractPortFromProxyPath(pathname: string): number | null {
		const match = pathname.match(/^\/api\/ports\/(\d+)\/proxy(?:\/.*)?$/);
		if (!match) return null;
		const port = Number(match[1]);
		if (!Number.isInteger(port) || port <= 0 || port > 65535) return null;
		return port;
	}

	function normalizeProxyUrl(raw: string, basePort: number): string | null {
		if (typeof window === 'undefined') return null;
		const input = raw.trim();
		if (!input) return null;

		let normalized: string;
		const base = getPreviewUrl(basePort);

		if (/^https?:\/\//i.test(input)) {
			normalized = input;
		} else if (input.startsWith('/api/ports/')) {
			normalized = `${window.location.origin}${input}`;
		} else if (input.startsWith('/')) {
			normalized = `${base}${input}`;
		} else if (input.startsWith('?') || input.startsWith('#')) {
			normalized = `${base}${input}`;
		} else {
			normalized = `${base}/${input}`;
		}

		let parsed: URL;
		try {
			parsed = new URL(normalized, window.location.origin);
		} catch {
			return null;
		}

		if (parsed.origin !== window.location.origin) {
			return null;
		}

		const targetPort = extractPortFromProxyPath(parsed.pathname);
		if (targetPort === null) {
			return null;
		}
		if (settings?.blocked_ports.includes(targetPort)) {
			return null;
		}
		if (!findPort(targetPort) && targetPort !== basePort) {
			return null;
		}

		return parsed.toString();
	}

	function canOpenEmbedded(port: PortEntry): boolean {
		if (!port.active) return false;
		if (port.is_previewable_http) return true;
		return settings?.allow_force_open_non_http ?? false;
	}

	function openPortInPreview(port: PortEntry, force = false) {
		if (!port.active) {
			error = `Port ${port.port} is not currently active.`;
			return;
		}
		if (!port.is_previewable_http && !force && !(settings?.allow_force_open_non_http ?? false)) {
			error = `Port ${port.port} is listening but did not pass HTTP probe.`;
			return;
		}

		const existing = previewTabs.find((tab) => tab.port === port.port);
		if (existing) {
			setPreviewState(previewTabs, existing.id);
			showNewTab = false;
			return;
		}

		const url = getPreviewUrl(port.port);
		const newTab: PreviewTab = {
			id: generateTabId(),
			port: port.port,
			title: port.name,
			history: [url],
			historyIndex: 0,
			isLoading: true,
			error: null
		};
		setPreviewState([...previewTabs, newTab], newTab.id);
		showNewTab = false;
	}

	function openNewTabPage() {
		showNewTab = true;
		activeTabId = null;
		syncPreviewAddress();
	}

	function openTabExternal(tabId: string) {
		const tab = previewTabs.find((t) => t.id === tabId);
		if (!tab) return;
		const url = tab.history[tab.historyIndex];
		window.open(url, '_blank', 'noopener,noreferrer');
	}

	function handleWindowClick(e: PointerEvent) {
		if (tabMenuOpen === null) return;
		const target = e.target as HTMLElement;
		if (!target.closest('[data-tab-menu]')) {
			tabMenuOpen = null;
		}
	}

	function setActiveTab(tabId: string) {
		if (!previewTabs.some((tab) => tab.id === tabId)) return;
		setPreviewState(previewTabs, tabId);
	}

	function closeTab(tabId: string) {
		const index = previewTabs.findIndex((tab) => tab.id === tabId);
		if (index === -1) return;
		const nextTabs = previewTabs.filter((tab) => tab.id !== tabId);
		if (nextTabs.length === 0) {
			setPreviewState([], null);
			return;
		}
		const nextActive =
			activeTabId === tabId
				? nextTabs[Math.max(0, index - 1)]?.id ?? nextTabs[0].id
				: activeTabId;
		setPreviewState(nextTabs, nextActive);
	}

	function updateTabNavigation(tabId: string, url: string, pushHistory: boolean) {
		const current = previewTabs.find((tab) => tab.id === tabId);
		if (!current) return;
		const normalized = normalizeProxyUrl(url, current.port);
		if (!normalized) {
			error = 'Invalid preview URL. Use /api/ports/{port}/proxy or a path within an open preview.';
			return;
		}

		const parsed = new URL(normalized);
		const targetPort = extractPortFromProxyPath(parsed.pathname);
		if (targetPort === null) {
			error = 'Invalid preview URL path.';
			return;
		}

		const matchedPort = findPort(targetPort);
		if (!matchedPort && targetPort !== current.port) {
			error = `Port ${targetPort} is not available in the current preview list.`;
			return;
		}

		const nextTabs = previewTabs.map((tab) => {
			if (tab.id !== tabId) return tab;
			let history = tab.history;
			let historyIndex = tab.historyIndex;
			if (pushHistory) {
				history = [...tab.history.slice(0, historyIndex + 1), normalized];
				historyIndex = history.length - 1;
			}
			return {
				...tab,
				port: targetPort,
				title: matchedPort?.name ?? `Port ${targetPort}`,
				history,
				historyIndex,
				isLoading: true,
				error: null
			};
		});

		setPreviewState(nextTabs, tabId);
	}

	function goBack() {
		const activeTab = getActiveTab();
		if (!activeTab || activeTab.historyIndex === 0) return;
		const nextTabs = previewTabs.map((tab) =>
			tab.id === activeTab.id
				? { ...tab, historyIndex: tab.historyIndex - 1, isLoading: true, error: null }
				: tab
		);
		setPreviewState(nextTabs, activeTab.id);
	}

	function goForward() {
		const activeTab = getActiveTab();
		if (!activeTab || activeTab.historyIndex >= activeTab.history.length - 1) return;
		const nextTabs = previewTabs.map((tab) =>
			tab.id === activeTab.id
				? { ...tab, historyIndex: tab.historyIndex + 1, isLoading: true, error: null }
				: tab
		);
		setPreviewState(nextTabs, activeTab.id);
	}

	function reloadActiveTab() {
		const activeTab = getActiveTab();
		if (!activeTab) return;
		const frame = getActiveFrame();
		if (frame?.contentWindow) {
			try {
				frame.contentWindow.location.reload();
				const nextTabs = previewTabs.map((tab) =>
					tab.id === activeTab.id ? { ...tab, isLoading: true, error: null } : tab
				);
				setPreviewState(nextTabs, activeTab.id);
				return;
			} catch {
				// Fallback below will still refresh by URL.
			}
		}
		updateTabNavigation(activeTab.id, activeTab.history[activeTab.historyIndex], false);
	}

	function navigateFromAddressBar() {
		const activeTab = getActiveTab();
		if (!activeTab) return;
		updateTabNavigation(activeTab.id, previewAddress, true);
	}

	function openActiveTabExternal() {
		const activeTab = getActiveTab();
		if (!activeTab) return;
		const url = activeTab.history[activeTab.historyIndex];
		window.open(url, '_blank', 'noopener,noreferrer');
	}

	function onIframeLoad(tabId: string) {
		const frame = previewFrames.get(tabId);
		const currentTab = previewTabs.find((tab) => tab.id === tabId);
		if (!currentTab) return;

		let nextUrl: string | null = null;
		if (frame?.contentWindow) {
			try {
				nextUrl = normalizeProxyUrl(frame.contentWindow.location.href, currentTab.port);
			} catch {
				nextUrl = null;
			}
		}

		const nextTabs = previewTabs.map((tab) => {
			if (tab.id !== tabId) return tab;
			let history = tab.history;
			let historyIndex = tab.historyIndex;
			if (nextUrl && nextUrl !== tab.history[tab.historyIndex]) {
				history = [...tab.history.slice(0, tab.historyIndex + 1), nextUrl];
				historyIndex = history.length - 1;
			}
			return {
				...tab,
				history,
				historyIndex,
				isLoading: false,
				error: null
			};
		});

		setPreviewState(nextTabs, activeTabId);
	}

	function onIframeError(tabId: string) {
		const nextTabs = previewTabs.map((tab) =>
			tab.id === tabId
				? { ...tab, isLoading: false, error: 'Preview failed to render in embedded mode.' }
				: tab
		);
		setPreviewState(nextTabs, activeTabId);
	}

	function syncTabTitles() {
		let changed = false;
		const nextTabs = previewTabs.map((tab) => {
			const matched = findPort(tab.port);
			if (!matched || matched.name === tab.title) {
				return tab;
			}
			changed = true;
			return { ...tab, title: matched.name };
		});
		if (changed) {
			setPreviewState(nextTabs, activeTabId);
		}
	}

	async function togglePin(port: PortEntry) {
		mutatingPort = port.port;
		try {
			if (port.pinned) {
				await apiClient.removePinnedPort(port.port);
			} else {
				await apiClient.addPinnedPort(port.port);
			}
			await loadPorts(true);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to update pinned ports';
		} finally {
			mutatingPort = null;
		}
	}

	async function hidePort(port: number) {
		mutatingPort = port;
		try {
			await apiClient.addHiddenPort(port);
			await loadPorts(true);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to hide port';
		} finally {
			mutatingPort = null;
		}
	}

	function probeStatusText(port: PortEntry): string {
		if (!port.active) return 'Offline';
		if (port.is_previewable_http) {
			return port.last_probe_ms ? `HTTP Ready (${port.last_probe_ms}ms)` : 'HTTP Ready';
		}
		switch (port.probe_status) {
			case 'timeout':
				return 'Probe Timeout';
			case 'conn_refused':
				return 'Connection Refused';
			case 'non_http':
				return 'Non-HTTP Listener';
			default:
				return 'Probe Failed';
		}
	}
</script>

<div class="flex h-full flex-col bg-background" onpointerdown={handleWindowClick} role="presentation">
	{#if loading}
		<div class="flex h-full items-center justify-center text-muted-foreground">
			<RefreshCw class="h-5 w-5 animate-spin" />
		</div>
	{:else if settings && !settings.enabled}
		<div class="flex h-full items-center justify-center p-6">
			<div class="max-w-xl rounded-xl border border-border bg-card p-5 text-center">
				<div class="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-muted">
					<AlertCircle class="h-6 w-6 text-muted-foreground" />
				</div>
				<h2 class="text-lg font-semibold">Preview Forwarding Disabled</h2>
				<p class="mt-2 text-sm text-muted-foreground">
					Enable preview forwarding in Menu &rarr; Settings &rarr; Preview &amp; Port Forwarding.
				</p>
			</div>
		</div>
	{:else}
		{#if error}
			<div class="flex items-start gap-2 border-b border-destructive/20 bg-destructive/10 px-3 py-2 text-sm text-destructive">
				<AlertCircle class="mt-0.5 h-4 w-4 shrink-0" />
				<span class="flex-1">{error}</span>
				<button onclick={() => (error = null)} class="shrink-0 p-0.5 hover:opacity-70">
					<X class="h-3.5 w-3.5" />
				</button>
			</div>
		{/if}

		{#if discoveryWarning}
			<div class="flex items-start gap-2 border-b border-amber-500/20 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400">
				<AlertCircle class="mt-0.5 h-4 w-4 shrink-0" />
				<span class="flex-1">{discoveryWarning}</span>
			</div>
		{/if}

		<!-- Tab Bar -->
		<div class="flex items-center gap-1 border-b border-border bg-card px-2 py-1.5">
			<div class="flex min-w-0 flex-1 gap-1 overflow-x-auto">
				{#each previewTabs as tab}
					<div class="relative shrink-0" data-tab-menu>
						<div
							class="inline-flex items-center rounded-md text-xs transition-colors {tab.id === activeTabId && !showNewTab
								? 'bg-primary/10 text-primary'
								: 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
						>
							<button
								class="inline-flex items-center gap-1 py-1.5 pl-2.5 pr-1"
								onclick={() => {
									setActiveTab(tab.id);
									showNewTab = false;
								}}
							>
								<span class="max-w-[120px] truncate">{tab.title}</span>
								<span class="font-mono opacity-60">:{tab.port}</span>
							</button>
							<button
								onclick={() => (tabMenuOpen = tabMenuOpen === tab.id ? null : tab.id)}
								class="rounded p-1 hover:bg-black/5 dark:hover:bg-white/10"
								aria-label="Tab options for {tab.title}"
							>
								<Ellipsis class="h-3.5 w-3.5" />
							</button>
							<button
								onclick={() => closeTab(tab.id)}
								class="rounded p-1 hover:bg-black/5 dark:hover:bg-white/10"
								aria-label="Close {tab.title}"
							>
								<X class="h-3 w-3" />
							</button>
						</div>

						<!-- Tab Dropdown Menu -->
						{#if tabMenuOpen === tab.id}
							<div class="absolute left-0 top-full z-50 mt-1 min-w-[160px] rounded-lg border border-border bg-card py-1 shadow-lg">
								{#if findPort(tab.port)}
									<button
										class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted"
										disabled={mutatingPort === tab.port}
										onclick={() => {
											const p = findPort(tab.port);
											if (p) togglePin(p);
											tabMenuOpen = null;
										}}
									>
										{#if findPort(tab.port)?.pinned}
											<PinOff class="h-3.5 w-3.5" />
											Unpin
										{:else}
											<Pin class="h-3.5 w-3.5" />
											Pin
										{/if}
									</button>
									<button
										class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted"
										disabled={mutatingPort === tab.port}
										onclick={() => {
											hidePort(tab.port);
											tabMenuOpen = null;
										}}
									>
										<EyeOff class="h-3.5 w-3.5" />
										Hide
									</button>
								{/if}
								<button
									class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted"
									onclick={() => {
										openTabExternal(tab.id);
										tabMenuOpen = null;
									}}
								>
									<ExternalLink class="h-3.5 w-3.5" />
									Open External
								</button>
								<button
									class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-muted"
									onclick={() => {
										copyPreviewUrl(tab.port);
										tabMenuOpen = null;
									}}
								>
									<Copy class="h-3.5 w-3.5" />
									{copiedPort === tab.port ? 'Copied!' : 'Copy URL'}
								</button>
								<div class="my-1 border-t border-border"></div>
								<button
									class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-destructive hover:bg-muted"
									onclick={() => {
										closeTab(tab.id);
										tabMenuOpen = null;
									}}
								>
									<X class="h-3.5 w-3.5" />
									Close Tab
								</button>
							</div>
						{/if}
					</div>
				{/each}
			</div>

			<button
				onclick={openNewTabPage}
				class="shrink-0 rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
				title="New tab"
			>
				<Plus class="h-4 w-4" />
			</button>

			<button
				onclick={() => loadPorts(true)}
				class="shrink-0 rounded-md p-1.5 text-muted-foreground hover:text-foreground"
				disabled={refreshing}
				title="Refresh ports"
			>
				<RefreshCw class="h-3.5 w-3.5 {refreshing ? 'animate-spin' : ''}" />
			</button>
		</div>

		<!-- Browser Controls Bar -->
		<div class="flex items-center gap-1.5 border-b border-border bg-card px-2 py-1.5">
			<button
				onclick={goBack}
				disabled={showNewTab || !getActiveTab() || (getActiveTab()?.historyIndex ?? 0) === 0}
				class="rounded-md p-1.5 hover:bg-muted disabled:opacity-40"
				title="Back"
			>
				<ChevronLeft class="h-4 w-4" />
			</button>
			<button
				onclick={goForward}
				disabled={showNewTab || !getActiveTab() || (getActiveTab()?.historyIndex ?? 0) >= (getActiveTab()?.history.length ?? 1) - 1}
				class="rounded-md p-1.5 hover:bg-muted disabled:opacity-40"
				title="Forward"
			>
				<ChevronRight class="h-4 w-4" />
			</button>
			<button
				onclick={reloadActiveTab}
				disabled={showNewTab || !getActiveTab()}
				class="rounded-md p-1.5 hover:bg-muted disabled:opacity-40"
				title="Reload"
			>
				<RotateCw class="h-4 w-4" />
			</button>

			<form
				onsubmit={(e) => {
					e.preventDefault();
					navigateFromAddressBar();
				}}
				class="flex min-w-0 flex-1 items-center"
			>
				<input
					type="text"
					bind:value={previewAddress}
					disabled={showNewTab || !getActiveTab()}
					placeholder={showNewTab || !getActiveTab() ? 'Select a tab or open a port' : '/api/ports/5173/proxy'}
					class="w-full rounded-md border border-input bg-background px-3 py-1 text-sm disabled:opacity-50"
				/>
			</form>

			<button
				onclick={openActiveTabExternal}
				disabled={showNewTab || !getActiveTab()}
				class="rounded-md p-1.5 hover:bg-muted disabled:opacity-40"
				title="Open in external browser"
			>
				<ExternalLink class="h-4 w-4" />
			</button>
		</div>

		<!-- Content Area -->
		<div class="relative min-h-0 flex-1">
			{#if showNewTab || previewTabs.length === 0}
				<!-- New Tab / Port Picker -->
				<div class="h-full overflow-y-auto p-4 md:p-6">
					<div class="mx-auto max-w-2xl">
						{#if ports.length === 0}
							<div class="flex flex-col items-center justify-center py-16 text-center">
								<div class="mb-3 text-sm text-muted-foreground">
									No servers detected. Start a dev server or pin a port.
								</div>
							</div>
						{:else}
							<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4">
								{#each ports as port}
									<button
										onclick={() => openPortInPreview(port, !port.is_previewable_http)}
										disabled={!canOpenEmbedded(port)}
										class="group flex flex-col items-start gap-1.5 rounded-lg border border-border p-3 text-left transition-colors hover:bg-muted disabled:opacity-50"
									>
										<div class="flex w-full items-center gap-2">
											<span
												class="h-2 w-2 shrink-0 rounded-full {port.is_previewable_http
													? 'bg-green-500'
													: port.active
														? 'bg-amber-500'
														: 'bg-muted-foreground/40'}"
											></span>
											<span class="truncate text-sm font-medium">{port.name}</span>
										</div>
										<span class="font-mono text-xs text-muted-foreground">localhost:{port.port}</span>
									</button>
								{/each}
							</div>
						{/if}
					</div>
				</div>
			{:else}
				<!-- Iframe Content -->
				{#each previewTabs as tab (tab.id)}
					<iframe
						use:registerPreviewFrame={tab.id}
						src={tab.history[tab.historyIndex]}
						title={`Preview: ${tab.title}`}
						class="h-full w-full border-0 {tab.id === activeTabId ? 'block' : 'hidden'}"
						onload={() => onIframeLoad(tab.id)}
						onerror={() => onIframeError(tab.id)}
					></iframe>
				{/each}

				{#if getActiveTab()?.isLoading}
					<div class="pointer-events-none absolute right-3 top-3 inline-flex items-center gap-1 rounded-full bg-card/95 px-3 py-1 text-xs text-muted-foreground shadow">
						<RefreshCw class="h-3.5 w-3.5 animate-spin" />
						Loading
					</div>
				{/if}

				{#if getActiveTab()?.error}
					<div class="absolute bottom-3 left-3 right-3 rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
						{getActiveTab()?.error}
					</div>
				{/if}
			{/if}
		</div>
	{/if}
</div>
