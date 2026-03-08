<script lang="ts">
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';
	import ChatView from '$components/chat/ChatView.svelte';
	import ChatHeader from '$components/chat/ChatHeader.svelte';
	import SessionSidebar from '$components/chat/SessionSidebar.svelte';
	import ModelSelector from '$components/chat/ModelSelector.svelte';
	import PlanTracker from '$components/chat/PlanTracker.svelte';
	import { sessionStore, loadSession, clearSession, setModel } from '$stores/session';
	import { sessionsStore, loadSessions, deleteSession } from '$stores/sessions';
	import { apiClient } from '$api/client';
	import { workspaceStore } from '$stores/workspace';

	const SIDEBAR_COLLAPSED_KEY = 'krusty:sidebar_collapsed';

	let defaultModel = $state('MiniMax-M2.5');
	let showModelSelector = $state(false);
	let sidebarCollapsed = $state(false);
	let mobileSidebarOpen = $state(false);

	// Get current model from session store, fallback to default
	let currentModel = $derived($sessionStore.model ?? defaultModel);

	// Load default model from server on mount
	async function loadDefaultModel() {
		try {
			const models = await apiClient.getModels();
			defaultModel = models.default_model;
		} catch (err) {
			console.error('Failed to load default model:', err);
		}
	}

	// Load sidebar state from localStorage
	function loadSidebarState() {
		if (browser) {
			const stored = localStorage.getItem(SIDEBAR_COLLAPSED_KEY);
			sidebarCollapsed = stored === 'true';
		}
	}

	function handleModelClick() {
		showModelSelector = true;
	}

	function handleModelSelect(modelId: string) {
		setModel(modelId);
	}

	function handleNewSession() {
		// Session is already initialized by ChatHeader's handleCreateSession
		// Just refresh the sessions list
		loadSessions();
	}

	function handleToggleCollapse() {
		sidebarCollapsed = !sidebarCollapsed;
		if (browser) {
			localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(sidebarCollapsed));
		}
	}

	let isPinching = $state(false);

	async function handlePinch() {
		if (!$sessionStore.sessionId) {
			return;
		}
		if (isPinching) return;

		isPinching = true;
		try {
			const result = await apiClient.pinchSession($sessionStore.sessionId);
			// Reload sessions to show the new child
			await loadSessions();
			// Load the new session
			await loadSession(result.session.id);
		} catch (err) {
			console.error('Pinch failed:', err);
			alert(err instanceof Error ? err.message : 'Failed to pinch session');
		} finally {
			isPinching = false;
		}
	}

	async function handleSelectSession(sessionId: string) {
		await loadSession(sessionId);
		// Close mobile sidebar after selection
		mobileSidebarOpen = false;
	}

	async function handleDeleteSession(sessionId: string) {
		await deleteSession(sessionId);
	}

	onMount(() => {
		const openModelListener = () => {
			showModelSelector = true;
		};

		void (async () => {
			loadSidebarState();
			await loadSessions();
			loadDefaultModel();

			// Restore last active session from workspace store
			const saved = workspaceStore.getState();
			if (saved.sessionId && !$sessionStore.sessionId) {
				await loadSession(saved.sessionId);
			}
		})();

		// Listen for model open event from ChatView AI controls
		window.addEventListener('openmodel', openModelListener);

		return () => {
			window.removeEventListener('openmodel', openModelListener);
		};
	});
</script>

<div class="flex h-full flex-col">
	<!-- Full-width header -->
	<ChatHeader
		{currentModel}
		{isPinching}
		onModelClick={handleModelClick}
		onNewSession={handleNewSession}
		onPinch={handlePinch}
		onHistoryClick={() => (mobileSidebarOpen = !mobileSidebarOpen)}
	/>

	<!-- Content area with sidebar rail and chat -->
	<div class="flex min-h-0 flex-1 relative">
		<!-- Mobile sidebar overlay -->
		{#if mobileSidebarOpen}
			<button
				onclick={() => (mobileSidebarOpen = false)}
				class="fixed inset-0 z-40 bg-black/50 md:hidden"
				aria-label="Close sidebar"
			></button>
		{/if}

		<!-- Mobile sidebar (slideover) - positioned below two-row header -->
		<div
			class="fixed left-0 z-50 w-72 transform transition-transform duration-200 md:hidden
				{mobileSidebarOpen ? 'translate-x-0' : '-translate-x-full'}"
			style="top: calc(6rem + var(--safe-area-top)); bottom: calc(4rem + var(--safe-area-bottom));"
		>
			<SessionSidebar
				currentSessionId={$sessionStore.sessionId}
				isCollapsed={false}
				onSelectSession={handleSelectSession}
				onDeleteSession={handleDeleteSession}
				onToggleCollapse={() => (mobileSidebarOpen = false)}
			/>
		</div>

		<!-- Desktop sidebar rail + panel -->
		<div class="hidden md:flex h-full">
			<SessionSidebar
				currentSessionId={$sessionStore.sessionId}
				isCollapsed={sidebarCollapsed}
				onSelectSession={handleSelectSession}
				onDeleteSession={handleDeleteSession}
				onToggleCollapse={handleToggleCollapse}
			/>
		</div>

		<!-- Main chat area -->
		<div class="flex min-h-0 flex-1 flex-col">
			<ChatView {currentModel} />
		</div>
	</div>
</div>

<!-- Model selector popup -->
<ModelSelector
	{currentModel}
	isOpen={showModelSelector}
	onClose={() => (showModelSelector = false)}
	onSelect={handleModelSelect}
/>

<!-- Plan tracker (floating) -->
<PlanTracker />
