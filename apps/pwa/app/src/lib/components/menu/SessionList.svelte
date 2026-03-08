<script lang="ts">
	import { onMount } from 'svelte';
	import { ArrowLeft, MessageSquare, Trash2, Plus, RefreshCw } from 'lucide-svelte';
	import { sessionsStore, loadSessions, selectSession, deleteSession, createSession } from '$stores/sessions';
	import { workspaceStore } from '$stores/workspace';

	interface Props {
		onBack: () => void;
	}

	let { onBack }: Props = $props();
	let deletingIds = $state<Set<string>>(new Set());

	onMount(() => {
		loadSessions();
	});

	async function handleDelete(id: string) {
		deletingIds = new Set([...deletingIds, id]);
		// Wait for fade animation
		await new Promise(r => setTimeout(r, 200));
		await deleteSession(id);
		deletingIds = new Set([...deletingIds].filter(x => x !== id));
	}

	function formatDate(dateStr: string) {
		const date = new Date(dateStr);
		const now = new Date();
		const diff = now.getTime() - date.getTime();
		const days = Math.floor(diff / (1000 * 60 * 60 * 24));

		if (days === 0) return 'Today';
		if (days === 1) return 'Yesterday';
		if (days < 7) return `${days} days ago`;
		return date.toLocaleDateString();
	}

	async function handleSelect(id: string) {
		await selectSession(id);
		// Navigate to chat
		window.location.href = '/';
	}

	async function handleNew() {
		// Use current workspace directory if set, otherwise create session without directory
		const currentDir = $workspaceStore.directory;
		await createSession(undefined, currentDir ?? undefined);
		window.location.href = '/';
	}
</script>

<div class="flex h-full flex-col">
	<!-- Header -->
	<div class="flex items-center gap-3 border-b border-border px-4 py-3">
		<button onclick={onBack} class="rounded-lg p-1 hover:bg-muted">
			<ArrowLeft class="h-5 w-5" />
		</button>
		<h2 class="flex-1 text-lg font-semibold">Sessions</h2>
		<button
			onclick={handleNew}
			class="flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-sm
				font-medium text-primary-foreground hover:bg-primary/90"
		>
			<Plus class="h-4 w-4" />
			New
		</button>
	</div>

	<!-- Sessions list -->
	<div class="flex-1 overflow-y-auto p-4">
		{#if $sessionsStore.isLoading}
			<div class="flex items-center justify-center py-8">
				<RefreshCw class="h-5 w-5 animate-spin text-muted-foreground" />
			</div>
		{:else if $sessionsStore.sessions.length === 0}
			<div class="py-8 text-center">
				<MessageSquare class="mx-auto mb-3 h-10 w-10 text-muted-foreground" />
				<p class="text-muted-foreground">No sessions yet</p>
			</div>
		{:else}
			<div class="space-y-2">
				{#each $sessionsStore.sessions as session}
					<div
						class="group flex items-center gap-3 rounded-xl bg-card p-3 transition-all duration-200
							{deletingIds.has(session.id) ? 'opacity-0 scale-95' : 'opacity-100'} hover:bg-muted"
					>
						<button
							onclick={() => handleSelect(session.id)}
							class="flex flex-1 items-center gap-3 text-left"
							disabled={deletingIds.has(session.id)}
						>
							<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-muted">
								<MessageSquare class="h-5 w-5 text-muted-foreground" />
							</div>
							<div class="min-w-0 flex-1">
								<div class="truncate font-medium">{session.title}</div>
								<div class="text-xs text-muted-foreground">
									{formatDate(session.updated_at)}
								</div>
							</div>
						</button>
						<button
							onclick={() => handleDelete(session.id)}
							disabled={deletingIds.has(session.id)}
							class="rounded-lg p-2 text-muted-foreground opacity-0 transition-opacity
								hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100
								disabled:pointer-events-none"
						>
							<Trash2 class="h-4 w-4" />
						</button>
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>
