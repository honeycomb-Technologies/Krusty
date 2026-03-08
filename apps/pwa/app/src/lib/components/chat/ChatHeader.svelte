<script lang="ts">
	import { Pencil, Check, Plus, GitBranch, X, Folder, ChevronUp, Loader2, History } from 'lucide-svelte';
	import { sessionStore, updateSessionTitle, initSession } from '$stores/session';
	import { createSession, getLastDirectory, loadDirectories, sessionsStore } from '$stores/sessions';
	import { gitStore, refreshGit, startGitPolling, stopGitPolling } from '$stores/git';
	import { apiClient } from '$api/client';
	import { onMount } from 'svelte';

	interface Props {
		currentModel: string;
		isPinching?: boolean;
		onModelClick: () => void;
		onNewSession: () => void;
		onPinch: () => void;
		onHistoryClick?: () => void;
	}

	let { currentModel, isPinching = false, onModelClick, onNewSession, onPinch, onHistoryClick }: Props = $props();

	// New session modal state
	let showNewSessionModal = $state(false);
	let selectedDirectory = $state<string | null>(null);
	let isCreating = $state(false);

	// Directory browser state
	let currentPath = $state<string>('');
	let parentPath = $state<string | null>(null);
	let directories = $state<{ name: string; path: string }[]>([]);
	let isLoadingDirs = $state(false);
	let dirError = $state<string | null>(null);
	let createError = $state<string | null>(null);

	type BranchMode = 'current' | 'existing' | 'new';
	let branchMode = $state<BranchMode>('current');
	let branchCurrent = $state<string | null>(null);
	let branchOptions = $state<string[]>([]);
	let existingBranch = $state<string>('');
	let newBranchName = $state('');
	let branchStatus = $state<'idle' | 'loading' | 'ready' | 'not_repo' | 'error'>('idle');
	let branchError = $state<string | null>(null);
	let branchLoadToken = 0;

	// Directory cache for instant navigation
	type DirCache = { current: string; parent: string | null; directories: { name: string; path: string }[] };
	const dirCache = new Map<string, DirCache>();

	// Scroll optimization
	let dirListContainer = $state<HTMLDivElement>(undefined!);
	let dirScrollTimeout: ReturnType<typeof setTimeout>;
	let pendingPrefetch: ReturnType<typeof setTimeout> | null = null;

	function handleDirScroll() {
		dirListContainer?.classList.add('dir-scrolling');
		clearTimeout(dirScrollTimeout);
		dirScrollTimeout = setTimeout(() => {
			dirListContainer?.classList.remove('dir-scrolling');
		}, 150);
	}

	onMount(() => {
		loadDirectories();
		// Pre-fetch home directory for instant first open
		prefetchDirectory();
		void refreshGit(true);
		startGitPolling();
		return () => {
			stopGitPolling();
			clearTimeout(dirScrollTimeout);
			if (pendingPrefetch) {
				clearTimeout(pendingPrefetch);
				pendingPrefetch = null;
			}
			dirCache.clear();
		};
	});

	async function prefetchDirectory(path?: string) {
		const cacheKey = path ?? '__home__';
		if (dirCache.has(cacheKey)) return;
		try {
			const result = await apiClient.browseDirectories(path);
			dirCache.set(cacheKey, result);
			// Also cache by actual path
			if (result.current !== cacheKey) {
				dirCache.set(result.current, result);
			}
		} catch {
			// Silent fail for prefetch
		}
	}

	async function loadDirectory(path?: string) {
		const cacheKey = path ?? '__home__';
		dirError = null;

		// Check cache first - instant display
		const cached = dirCache.get(cacheKey) ?? dirCache.get(path ?? '');
		if (cached) {
			currentPath = cached.current;
			parentPath = cached.parent;
			directories = cached.directories;
			// Background refresh
			refreshDirectory(path);
			return;
		}

		// No cache - show loading
		isLoadingDirs = true;
		try {
			const result = await apiClient.browseDirectories(path);
			currentPath = result.current;
			parentPath = result.parent;
			directories = result.directories;
			// Cache it
			dirCache.set(cacheKey, result);
			dirCache.set(result.current, result);
		} catch (e) {
			dirError = e instanceof Error ? e.message : 'Failed to load';
		} finally {
			isLoadingDirs = false;
		}
	}

	async function refreshDirectory(path?: string) {
		try {
			const result = await apiClient.browseDirectories(path);
			const cacheKey = path ?? '__home__';
			dirCache.set(cacheKey, result);
			dirCache.set(result.current, result);
			// Update UI if still on same path
			if (currentPath === result.current) {
				directories = result.directories;
				parentPath = result.parent;
			}
		} catch {
			// Silent fail for refresh
		}
	}

	function resetBranchState() {
		branchMode = 'current';
		branchCurrent = null;
		branchOptions = [];
		existingBranch = '';
		newBranchName = '';
		branchStatus = 'idle';
		branchError = null;
	}

	async function loadBranchContext(path: string | null) {
		const token = ++branchLoadToken;

		if (!path) {
			resetBranchState();
			return;
		}

		branchStatus = 'loading';
		branchError = null;

		try {
			const status = await apiClient.getGitStatus(path);
			if (token !== branchLoadToken) return;

			if (!status.in_repo) {
				resetBranchState();
				branchStatus = 'not_repo';
				return;
			}

			branchCurrent = status.branch;
			const branches = await apiClient.getGitBranches(path);
			if (token !== branchLoadToken) return;

			branchOptions = branches.branches
				.filter((branch) => !branch.is_remote)
				.map((branch) => branch.name);

			existingBranch = branchCurrent ?? branchOptions[0] ?? '';
			branchStatus = 'ready';
		} catch (err) {
			if (token !== branchLoadToken) return;
			branchStatus = 'error';
			branchError = err instanceof Error ? err.message : 'Failed to load git branches';
		}
	}

	async function handleCreateSession() {
		if (isCreating) return;
		isCreating = true;
		createError = null;
		try {
			let targetBranch: string | undefined;
			const directory = selectedDirectory ?? undefined;

			if (directory && branchStatus === 'ready') {
				if (branchMode === 'current') {
					targetBranch = branchCurrent ?? undefined;
				} else if (branchMode === 'existing') {
					targetBranch = existingBranch || branchCurrent || undefined;
					if (targetBranch) {
						await apiClient.checkoutGitBranch(targetBranch, directory, false);
					}
				} else if (branchMode === 'new') {
					const branch = newBranchName.trim();
					if (!branch) {
						createError = 'Enter a branch name or switch branch mode.';
						return;
					}
					const create = !branchOptions.includes(branch);
					await apiClient.checkoutGitBranch(branch, directory, create);
					targetBranch = branch;
				}
			}

			const newSession = await createSession(undefined, directory, targetBranch);
			if (newSession) {
				// Initialize the current session with the new ID and title
				initSession(newSession.id, newSession.title);
			}
			showNewSessionModal = false;
			onNewSession();
			await refreshGit(true);
		} catch (err) {
			createError = err instanceof Error ? err.message : 'Failed to create session';
		} finally {
			isCreating = false;
		}
	}

	async function openNewSessionModal() {
		dirCache.clear();
		resetBranchState();
		createError = null;
		selectedDirectory = getLastDirectory();
		showNewSessionModal = true;
		// Load initial directory
		await loadDirectory(selectedDirectory ?? undefined);
		await loadBranchContext(selectedDirectory);
		// Pre-fetch parent and first few subdirs for instant navigation
		if (parentPath) prefetchDirectory(parentPath);
		directories.slice(0, 5).forEach(d => prefetchDirectory(d.path));
	}

	function closeModal() {
		createError = null;
		showNewSessionModal = false;
	}

	// Navigation = Selection: wherever you navigate becomes selected
	function navigateTo(path: string) {
		if (pendingPrefetch) clearTimeout(pendingPrefetch);
		selectedDirectory = path;
		loadDirectory(path);
		void loadBranchContext(path);
		// Pre-fetch visible subdirectories for instant next click
		pendingPrefetch = setTimeout(() => {
			const cached = dirCache.get(path);
			if (cached) {
				cached.directories.slice(0, 5).forEach(d => prefetchDirectory(d.path));
			}
			pendingPrefetch = null;
		}, 50);
	}

	function navigateUp() {
		if (parentPath) {
			selectedDirectory = parentPath;
			loadDirectory(parentPath);
			void loadBranchContext(parentPath);
		}
	}

	function jumpToRecent(dir: string) {
		selectedDirectory = dir;
		loadDirectory(dir);
		void loadBranchContext(dir);
	}

	function formatTokens(count: number): string {
		if (count >= 1000000) return `${(count / 1000000).toFixed(1)}M`;
		if (count >= 1000) return `${(count / 1000).toFixed(1)}K`;
		return count.toString();
	}

	const CONTEXT_LIMIT = 200000;

	function getContextStatus(tokens: number): { color: string; label: string } {
		const pct = (tokens / CONTEXT_LIMIT) * 100;
		if (pct >= 90) return { color: 'text-red-500', label: 'CRITICAL' };
		if (pct >= 75) return { color: 'text-orange-500', label: 'HIGH' };
		if (pct >= 50) return { color: 'text-yellow-500', label: '' };
		return { color: 'text-muted-foreground', label: '' };
	}

	let isEditingTitle = $state(false);
	let editedTitle = $state('');
	let titleInput = $state<HTMLInputElement>(undefined!);

	function startEditTitle() {
		editedTitle = $sessionStore.title;
		isEditingTitle = true;
		setTimeout(() => titleInput?.focus(), 0);
	}

	function saveTitle() {
		const newTitle = editedTitle.trim() || 'Untitled';
		isEditingTitle = false;
		if (newTitle !== $sessionStore.title && $sessionStore.sessionId) {
			updateSessionTitle($sessionStore.sessionId, newTitle);
		}
	}

	function handleTitleKeyDown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			saveTitle();
		} else if (e.key === 'Escape') {
			isEditingTitle = false;
		}
	}

	function handleModalKeyDown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeModal();
	}

	function getShortPath(path: string | null): string {
		if (!path) return 'No directory';
		const parts = path.split('/').filter(Boolean);
		return parts.slice(-2).join('/') || path;
	}

	function dirtyFileCount(): number {
		if (!$gitStore.status?.in_repo) return 0;
		return $gitStore.status.total_changes;
	}

	function currentBranchLabel(): string {
		if (!$gitStore.status?.in_repo) return 'No repo';
		return $gitStore.status.branch ?? 'detached';
	}
</script>

<svelte:window on:keydown={showNewSessionModal ? handleModalKeyDown : undefined} />

<!--
  Mobile (< md): Two rows
    - Row 1: History | Wave + Title (flex) | Pinch | New chat
    - Row 2: Git status left | Context right
  Desktop (md+): Single row, original layout
-->
<header class="relative z-50 flex flex-col md:flex-row md:items-center md:justify-between shrink-0 border-b border-border/50 bg-card/60 backdrop-blur-sm px-4 md:h-14">
	<!-- ============================================
	     MOBILE ROW 1: History | Title | Pinch | New chat
	     ============================================ -->
	<div class="flex items-center justify-between py-2 md:hidden md:py-0">
		<!-- Left: History -->
		<div class="flex items-center">
			{#if onHistoryClick}
				<button
					onclick={onHistoryClick}
					class="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					title="Session history"
				>
					<History class="h-5 w-5" />
				</button>
			{/if}
		</div>

		<!-- Center: Title + Wave -->
		<div class="relative flex items-center min-w-0 flex-1 justify-center">
			<!-- Wave indicator: absolutely positioned left, fills gap without shifting title -->
			{#if $sessionStore.isStreaming}
				<div class="absolute left-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5 h-4 pointer-events-none" title="Streaming">
					<span class="w-1 h-1 rounded-full bg-green-500 animate-wave"></span>
					<span class="w-1 h-1.5 rounded-full bg-green-500 animate-wave" style="animation-delay: 0.1s"></span>
					<span class="w-1 h-2 rounded-full bg-green-500 animate-wave" style="animation-delay: 0.2s"></span>
					<span class="w-1 h-1.5 rounded-full bg-green-500 animate-wave" style="animation-delay: 0.3s"></span>
					<span class="w-1 h-1 rounded-full bg-green-500 animate-wave" style="animation-delay: 0.4s"></span>
				</div>
			{/if}

			{#if !$sessionStore.sessionId}
				<span class="text-sm text-muted-foreground">No session</span>
			{:else if isEditingTitle}
				<input
					bind:this={titleInput}
					bind:value={editedTitle}
					onkeydown={handleTitleKeyDown}
					onblur={saveTitle}
					class="w-full min-w-[120px] rounded border border-input bg-background px-2 py-1 text-sm font-medium focus:outline-none focus:ring-2 focus:ring-ring"
				/>
				<button onclick={saveTitle} class="rounded p-1 text-green-500 hover:bg-muted shrink-0">
					<Check class="h-4 w-4" />
				</button>
			{:else}
				<button
					onclick={startEditTitle}
					class="group flex items-center gap-2 rounded-lg px-2 py-1 text-sm font-medium transition-colors hover:bg-muted min-w-0"
				>
					<span class="truncate">{$sessionStore.title}</span>
					<Pencil class="h-3 w-3 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 shrink-0" />
				</button>
			{/if}
		</div>

		<!-- Right: Pinch + New chat -->
		<div class="flex items-center gap-1">
			<!-- Pinch button -->
				<button
					onclick={onPinch}
					disabled={!$sessionStore.sessionId || isPinching}
					class="rounded p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
					title="Pinch (continue session)"
				>
				{#if isPinching}
					<span class="h-4 w-4 animate-spin rounded-full border border-current border-t-transparent"></span>
				{:else}
					<GitBranch class="h-4 w-4" />
				{/if}
			</button>

			<!-- New session button -->
			<button
				onclick={openNewSessionModal}
				class="rounded p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
				title="New session"
			>
				<Plus class="h-4 w-4" />
			</button>
		</div>
	</div>

	<!-- ============================================
	     MOBILE ROW 2: Git status (left) | Context (right)
	     ============================================ -->
	<div class="flex items-center justify-between border-t border-border/30 py-1.5 md:hidden md:border-none md:py-0">
		<!-- Left: Git status -->
		<div class="flex items-center gap-2">
			{#if $gitStore.status?.in_repo}
				<span class="inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-1.5 py-0.5 text-xs text-muted-foreground">
					Dirty: {dirtyFileCount()}
				</span>
				<span class="inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-1.5 py-0.5 text-xs text-muted-foreground">
					Branch: {currentBranchLabel()}
				</span>
			{:else}
				<span class="inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-1.5 py-0.5 text-xs text-muted-foreground">
					No repo
				</span>
			{/if}

			{#if $gitStore.isLoading}
				<Loader2 class="h-3 w-3 animate-spin text-muted-foreground" />
			{/if}
		</div>

		<!-- Right: Context -->
		<div class="flex items-center">
			{#if $sessionStore.tokenCount > 0}
				{@const status = getContextStatus($sessionStore.tokenCount)}
				<span class="text-xs {status.color}" title="Context usage">
					{#if status.label}
						<span class="font-semibold">{status.label}</span>
					{/if}
					{Math.round($sessionStore.tokenCount / CONTEXT_LIMIT * 100)}%
				</span>
			{/if}
		</div>
	</div>

	<!-- ============================================
	     DESKTOP ONLY: Full header row (hidden on mobile)
	     ============================================ -->
	<div class="hidden md:flex items-center justify-between w-full">
		<!-- Left: History -->
		<div class="flex items-center">
			{#if onHistoryClick}
				<button
					onclick={onHistoryClick}
					class="rounded-lg p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					title="Session history"
				>
					<History class="h-5 w-5" />
				</button>
			{/if}
		</div>

			<!-- Center: Git info + Title -->
			<div class="relative flex items-center gap-2 flex-1 justify-center">
				<!-- Git info -->
				<div class="flex items-center gap-2">
					{#if $gitStore.status?.in_repo}
						<span class="hidden sm:inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-xs text-muted-foreground">
							Dirty: {dirtyFileCount()}
						</span>
						<span class="hidden md:inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-xs text-muted-foreground">
							Branch: {currentBranchLabel()}
						</span>
					{:else}
						<span class="hidden sm:inline-flex items-center rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-xs text-muted-foreground">
							No repo
						</span>
					{/if}

					{#if $gitStore.isLoading}
						<Loader2 class="h-4 w-4 animate-spin text-muted-foreground" />
					{/if}
				</div>

			<!-- Title -->
			{#if $sessionStore.sessionId}
				{#if isEditingTitle}
					<input
						bind:this={titleInput}
						bind:value={editedTitle}
						onkeydown={handleTitleKeyDown}
						onblur={saveTitle}
						class="w-48 rounded border border-input bg-background px-2 py-1 text-sm font-medium focus:outline-none focus:ring-2 focus:ring-ring"
					/>
					<button onclick={saveTitle} class="rounded p-1 text-green-500 hover:bg-muted">
						<Check class="h-4 w-4" />
					</button>
				{:else}
					<button
						onclick={startEditTitle}
						class="group flex items-center gap-2 rounded-lg px-2 py-1 text-sm font-medium transition-colors hover:bg-muted"
					>
						<span class="max-w-[200px] truncate">{$sessionStore.title}</span>
						<Pencil class="h-3 w-3 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
					</button>
				{/if}

				{#if $sessionStore.isStreaming}
					<span class="absolute left-2 top-1/2 -translate-y-1/2 flex items-center gap-2 text-sm text-muted-foreground pointer-events-none">
						<span class="h-2 w-2 animate-pulse rounded-full bg-green-500"></span>
						{$sessionStore.isThinking ? 'Thinking...' : 'Streaming...'}
					</span>
				{/if}
			{/if}

			{#if $sessionStore.tokenCount > 0}
				{@const status = getContextStatus($sessionStore.tokenCount)}
				<span class="text-sm {status.color}" title="Context usage: {Math.round($sessionStore.tokenCount / CONTEXT_LIMIT * 100)}% of {formatTokens(CONTEXT_LIMIT)} limit">
					{#if status.label}
						<span class="font-semibold">{status.label}</span>
					{/if}
					{formatTokens($sessionStore.tokenCount)} / {formatTokens(CONTEXT_LIMIT)}
				</span>
			{/if}

			<div class="hidden sm:block h-4 w-px bg-border"></div>
		</div>

		<!-- Right: Pinch + New chat -->
		<div class="flex items-center gap-2">
			<!-- Pinch button -->
				<button
					onclick={onPinch}
					disabled={!$sessionStore.sessionId || isPinching}
					class="rounded p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
					title="Pinch (continue session)"
				>
				{#if isPinching}
					<span class="h-4 w-4 animate-spin rounded-full border border-current border-t-transparent"></span>
				{:else}
					<GitBranch class="h-4 w-4" />
				{/if}
			</button>

			<!-- New session button -->
			<button
				onclick={openNewSessionModal}
				class="rounded p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
				title="New session"
			>
				<Plus class="h-4 w-4" />
			</button>
		</div>
	</div>
</header>

<!-- New Session Modal with integrated directory browser -->
{#if showNewSessionModal}
	<button
		class="fixed inset-0 z-50 bg-black/60"
		onclick={closeModal}
		aria-label="Close modal"
	></button>

	<div class="fixed left-1/2 top-1/2 z-50 flex max-h-[80vh] w-full max-w-lg -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl border border-border/50 bg-card shadow-2xl">
		<!-- Header with current path (navigation = selection) -->
		<div class="flex shrink-0 items-center gap-2 border-b border-border px-4 py-3 bg-muted/30">
			{#if parentPath}
				<button onclick={navigateUp} class="rounded p-1.5 hover:bg-muted" title="Go up">
					<ChevronUp class="h-4 w-4" />
				</button>
			{:else}
				<Folder class="h-4 w-4 text-muted-foreground" />
			{/if}
			<span class="text-sm font-medium truncate flex-1" title={selectedDirectory ?? 'No directory'}>
				{selectedDirectory ?? 'Select a directory'}
			</span>
			<button onclick={closeModal} class="rounded p-1 text-muted-foreground hover:bg-muted">
				<X class="h-5 w-5" />
			</button>
		</div>

		<!-- Directory list -->
		<div bind:this={dirListContainer} onscroll={handleDirScroll} class="dir-scroll-container min-h-0 flex-1 overflow-y-auto">
			{#if isLoadingDirs}
				<div class="flex items-center justify-center py-8">
					<Loader2 class="h-5 w-5 animate-spin text-muted-foreground" />
				</div>
			{:else if dirError}
				<div class="px-4 py-8 text-center text-sm text-red-500">{dirError}</div>
			{:else if directories.length === 0}
				<div class="px-4 py-8 text-center text-sm text-muted-foreground">No subdirectories</div>
			{:else}
				{#each directories as dir (dir.path)}
					<button onclick={() => navigateTo(dir.path)} class="dir-item">
						<Folder class="h-4 w-4 shrink-0 text-muted-foreground" />
						<span class="truncate">{dir.name}</span>
					</button>
				{/each}
			{/if}
		</div>

		<!-- Recent directories -->
		{#if $sessionsStore.directories.length > 0}
			<div class="shrink-0 border-t border-border">
				<div class="px-4 py-1 text-xs font-medium text-muted-foreground bg-muted/30">Recent</div>
				<div class="max-h-[80px] overflow-y-auto">
					{#each $sessionsStore.directories as dir (dir)}
						<button onclick={() => jumpToRecent(dir)} class="recent-item {selectedDirectory === dir ? 'bg-primary/10 text-primary' : ''}">
							<Check class="h-3 w-3 shrink-0 {selectedDirectory === dir ? 'opacity-100' : 'opacity-0'}" />
							<span class="truncate text-muted-foreground">{getShortPath(dir)}</span>
						</button>
					{/each}
				</div>
			</div>
		{/if}

		<!-- Optional branch setup -->
		{#if selectedDirectory}
			<div class="shrink-0 border-t border-border">
				<div class="px-4 py-1 text-xs font-medium text-muted-foreground bg-muted/30">Branch setup (optional)</div>
				<div class="p-3 space-y-2">
					{#if branchStatus === 'loading'}
						<div class="flex items-center gap-2 text-sm text-muted-foreground">
							<Loader2 class="h-4 w-4 animate-spin" />
							<span>Loading git branches...</span>
						</div>
					{:else if branchStatus === 'not_repo'}
						<p class="text-sm text-muted-foreground">This directory is not a git repository.</p>
					{:else if branchStatus === 'error'}
						<p class="text-sm text-red-500">{branchError}</p>
					{:else if branchStatus === 'ready'}
						<label class="block text-xs text-muted-foreground">Session branch behavior</label>
						<select
							class="w-full rounded border border-input bg-background px-2 py-1.5 text-sm"
							bind:value={branchMode}
						>
							<option value="current">Keep current branch ({branchCurrent ?? 'detached'})</option>
							<option value="existing">Checkout existing branch</option>
							<option value="new">Create or checkout branch</option>
						</select>

						{#if branchMode === 'existing'}
							<select
								class="w-full rounded border border-input bg-background px-2 py-1.5 text-sm"
								bind:value={existingBranch}
								disabled={branchOptions.length === 0}
							>
								{#if branchOptions.length === 0}
									<option value="">No local branches available</option>
								{:else}
									{#each branchOptions as branch (branch)}
										<option value={branch}>{branch}</option>
									{/each}
								{/if}
							</select>
						{:else if branchMode === 'new'}
							<input
								type="text"
								placeholder="feature/new-work"
								bind:value={newBranchName}
								class="w-full rounded border border-input bg-background px-2 py-1.5 text-sm"
							/>
							<p class="text-xs text-muted-foreground">
								If the branch exists, Krusty checks it out. If not, it creates it from current HEAD.
							</p>
						{/if}
					{/if}
				</div>
			</div>
		{/if}

		<!-- Actions -->
		<div class="shrink-0 flex items-center justify-between gap-2 border-t border-border p-3 bg-muted/20">
			<button
				onclick={() => {
					selectedDirectory = null;
					resetBranchState();
					handleCreateSession();
				}}
				class="rounded-lg px-3 py-2 text-sm text-muted-foreground hover:bg-muted"
			>
				No Directory
			</button>
			<div class="flex gap-2">
				<button onclick={closeModal} class="rounded-lg px-4 py-2 text-sm text-muted-foreground hover:bg-muted">
					Cancel
				</button>
				<button
					onclick={handleCreateSession}
					disabled={isCreating || branchStatus === 'loading'}
					class="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
				>
					{isCreating ? 'Creating...' : 'Create'}
				</button>
			</div>
		</div>
		{#if createError}
			<div class="shrink-0 px-4 pb-3 text-sm text-red-500">{createError}</div>
		{/if}
	</div>
{/if}

<style>
	.dir-scroll-container {
		-webkit-overflow-scrolling: touch;
		overscroll-behavior: contain;
	}
	.dir-item {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		width: calc(100% - 1rem);
		height: 40px;
		margin: 0 0.5rem;
		padding: 0 0.75rem;
		border-radius: 0.5rem;
		font-size: 0.875rem;
		text-align: left;
		contain: layout style paint;
		content-visibility: auto;
		contain-intrinsic-size: auto 40px;
	}
	.dir-item:hover {
		background-color: hsl(var(--muted));
	}
	.recent-item {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		width: 100%;
		height: 32px;
		padding: 0 1rem;
		font-size: 0.875rem;
		text-align: left;
		contain: layout style paint;
	}
	.recent-item:hover {
		background-color: hsl(var(--muted));
	}
	:global(.dir-scrolling .dir-item),
	:global(.dir-scrolling .recent-item) {
		pointer-events: none !important;
	}

	/* Wave animation for streaming indicator */
	@keyframes wave {
		0%, 100% { transform: scaleY(0.5); }
		50% { transform: scaleY(1); }
	}
</style>
