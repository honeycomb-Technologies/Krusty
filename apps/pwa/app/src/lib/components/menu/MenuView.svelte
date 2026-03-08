<script lang="ts">
	import Settings from './Settings.svelte';
	import ProvidersView from './ProvidersView.svelte';
	import McpView from './McpView.svelte';
	import ProcessesView from './ProcessesView.svelte';
	import AsciiTitle from '../chat/AsciiTitle.svelte';
	import { ChevronRight, Key, Plug, Activity, Settings as SettingsIcon } from 'lucide-svelte';

	type View = 'main' | 'settings' | 'providers' | 'mcp' | 'processes';
	let currentView = $state<View>('main');

	interface MenuItem {
		id: View;
		label: string;
		description: string;
		icon: typeof Key;
	}

	const menuItems: MenuItem[] = [
		{ id: 'providers', label: 'AI Providers', description: 'Manage API keys and credentials', icon: Key },
		{ id: 'mcp', label: 'MCP Servers', description: 'Connect to Model Context Protocol servers', icon: Plug },
		{ id: 'processes', label: 'Background Tasks', description: 'View running processes', icon: Activity },
		{ id: 'settings', label: 'Settings', description: 'Configure app preferences', icon: SettingsIcon }
	];
</script>

<div class="h-full overflow-y-auto">
	{#if currentView === 'main'}
		<div class="p-4">
			<!-- Header -->
			<div class="mb-6 flex flex-col items-center">
				<AsciiTitle />
				<p class="mt-2 text-sm text-muted-foreground">v0.5.4</p>
			</div>

			<!-- Menu items -->
			<div class="space-y-2">
				{#each menuItems as item}
					<button
						onclick={() => (currentView = item.id)}
						class="flex w-full items-center justify-between rounded-xl bg-card p-4
							text-left transition-colors hover:bg-muted"
					>
						<div class="flex items-center gap-3">
							<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-muted">
								<item.icon class="h-5 w-5 text-muted-foreground" />
							</div>
							<div>
								<div class="font-medium">{item.label}</div>
								<div class="text-sm text-muted-foreground">{item.description}</div>
							</div>
						</div>
						<ChevronRight class="h-5 w-5 text-muted-foreground" />
					</button>
				{/each}
			</div>

			<!-- Quick info -->
			<div class="mt-8 rounded-xl bg-muted/50 p-4">
				<h3 class="mb-2 text-sm font-medium">About</h3>
				<p class="text-xs text-muted-foreground">
					Krusty is your local-first AI coding assistant. Connect this app to any self-hosted
					`krusty-server` endpoint.
				</p>
			</div>
		</div>
	{:else if currentView === 'providers'}
		<ProvidersView onBack={() => (currentView = 'main')} />
	{:else if currentView === 'mcp'}
		<McpView onBack={() => (currentView = 'main')} />
	{:else if currentView === 'processes'}
		<ProcessesView onBack={() => (currentView = 'main')} />
	{:else if currentView === 'settings'}
		<Settings onBack={() => (currentView = 'main')} />
	{/if}
</div>
