<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import {
		ArrowLeft,
		Check,
		X,
		Eye,
		EyeOff,
		Loader2,
		LogIn,
		LogOut,
		ExternalLink
	} from 'lucide-svelte';
	import {
		apiClient,
		type OAuthDeviceCodeInfo,
		type ProviderStatus
	} from '$lib/api/client';

	interface Props {
		onBack: () => void;
	}

	type OAuthDialog =
		| {
				kind: 'browser_callback';
				providerId: string;
				url: string;
		  }
		| {
				kind: 'device';
				providerId: string;
				url: string;
				deviceCode: OAuthDeviceCodeInfo;
		  }
		| {
				kind: 'paste_code';
				providerId: string;
				url: string;
		  };

	let { onBack }: Props = $props();

	let providers = $state<ProviderStatus[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	// API key editing
	let editingProvider = $state<string | null>(null);
	let apiKeyInput = $state('');
	let showApiKey = $state(false);
	let saving = $state(false);
	let apiKeyInputEl = $state<HTMLInputElement>(undefined!);

	// OAuth state
	let oauthLoading = $state<string | null>(null);
	let pasteCodeInput = $state('');
	let pasteCodeProvider = $state<string | null>(null);
	let oauthPollingInterval = $state<ReturnType<typeof setInterval> | null>(null);
	let pasteCodeInputEl = $state<HTMLInputElement>(undefined!);
	let oauthBridgeChannel: BroadcastChannel | null = null;

	let oauthDialog = $state<OAuthDialog | null>(null);

	onMount(() => {
		window.addEventListener('message', handleOAuthMessage);
		window.addEventListener('storage', handleOAuthStorage);
		if (typeof BroadcastChannel !== 'undefined') {
			oauthBridgeChannel = new BroadcastChannel('krusty:oauth');
			oauthBridgeChannel.onmessage = (event) => {
				void completeOAuthFromBrowser(event.data);
			};
		}
		void consumeStoredOAuthResult();
		loadProviders();
	});

	onDestroy(() => {
		stopPolling();
		window.removeEventListener('message', handleOAuthMessage);
		window.removeEventListener('storage', handleOAuthStorage);
		if (oauthBridgeChannel) {
			oauthBridgeChannel.close();
			oauthBridgeChannel = null;
		}
	});

	function stopPolling() {
		if (oauthPollingInterval) {
			clearInterval(oauthPollingInterval);
			oauthPollingInterval = null;
		}
	}

	async function loadProviders() {
		loading = true;
		error = null;
		try {
			providers = await apiClient.getCredentials();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load providers';
		} finally {
			loading = false;
		}
	}

	async function saveApiKey(providerId: string) {
		if (!apiKeyInput.trim()) return;

		saving = true;
		try {
			await apiClient.setCredential(providerId, apiKeyInput);
			providers = providers.map((p) =>
				p.id === providerId ? { ...p, configured: true } : p
			);
			editingProvider = null;
			apiKeyInput = '';
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to save';
		} finally {
			saving = false;
		}
	}

	async function removeApiKey(providerId: string) {
		try {
			await apiClient.deleteCredential(providerId);
			providers = providers.map((p) =>
				p.id === providerId ? { ...p, configured: false } : p
			);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to remove';
		}
	}

	function startEditing(providerId: string) {
		editingProvider = providerId;
		apiKeyInput = '';
		showApiKey = false;
		pasteCodeProvider = null;
		setTimeout(() => apiKeyInputEl?.focus(), 0);
	}

	function cancelEditing() {
		editingProvider = null;
		apiKeyInput = '';
	}

	async function startOAuth(providerId: string) {
		oauthLoading = providerId;
		error = null;
		try {
			const result = await apiClient.startOAuth(providerId);
			editingProvider = null;
			pasteCodeProvider = null;
			pasteCodeInput = '';

			if (result.flow_type === 'browser_callback') {
				oauthDialog = {
					kind: 'browser_callback',
					providerId,
					url: result.auth_url
				};
				openOAuthInNewTab();
				startPolling(providerId);
				return;
			}

			if (result.flow_type === 'device' && result.device_code) {
				oauthDialog = {
					kind: 'device',
					providerId,
					url: result.auth_url,
					deviceCode: result.device_code
				};
				openOAuthInNewTab();
				startPolling(providerId);
				return;
			}

			if (result.flow_type === 'paste_code' || result.paste_code) {
				pasteCodeProvider = providerId;
				oauthDialog = {
					kind: 'paste_code',
					providerId,
					url: result.auth_url
				};
				openOAuthInNewTab();
				oauthLoading = null;
				setTimeout(() => pasteCodeInputEl?.focus(), 100);
				return;
			}

			throw new Error('Unsupported OAuth flow response');
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to start OAuth';
			oauthLoading = null;
		}
	}

	function closeOAuthDialog() {
		if (oauthDialog?.kind === 'paste_code') {
			oauthLoading = null;
			pasteCodeProvider = null;
			pasteCodeInput = '';
		}
		oauthDialog = null;
	}

	function openOAuthInNewTab(url = oauthDialog?.url) {
		if (!url) return;
		const popup = window.open(url, '_blank', 'noopener,noreferrer');
		if (!popup) {
			error = 'Your browser blocked the sign-in window. Use "Open browser" or allow popups.';
		}
	}

	function startPolling(providerId: string) {
		stopPolling();
		oauthPollingInterval = setInterval(async () => {
			try {
				const status = await apiClient.getOAuthStatus(providerId);
				if (status.has_token) {
					await finishOAuthSuccess(providerId);
				} else if (!status.flow_active) {
					stopPolling();
					if (oauthLoading === providerId) {
						error = 'Sign-in was cancelled or expired before completion';
					}
					oauthLoading = null;
					oauthDialog = null;
					pasteCodeProvider = null;
				}
			} catch {
				// Ignore polling errors
			}
		}, 2000);
	}

	async function submitPasteCode() {
		if (!pasteCodeInput.trim() || !pasteCodeProvider) return;

		oauthLoading = pasteCodeProvider;
		error = null;
		try {
			await apiClient.exchangeOAuthCode(pasteCodeProvider, pasteCodeInput.trim());
			pasteCodeProvider = null;
			pasteCodeInput = '';
			oauthLoading = null;
			oauthDialog = null;
			await loadProviders();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to exchange code';
			oauthLoading = null;
		}
	}

	function cancelPasteCode() {
		pasteCodeProvider = null;
		pasteCodeInput = '';
		oauthDialog = null;
	}

	async function revokeOAuth(providerId: string) {
		try {
			await apiClient.revokeOAuth(providerId);
			providers = providers.map((p) =>
				p.id === providerId ? { ...p, has_oauth: false } : p
			);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to sign out';
		}
	}

	function statusText(provider: ProviderStatus): string {
		if (provider.has_oauth && provider.configured) return 'OAuth + API Key';
		if (provider.has_oauth) return 'OAuth connected';
		if (provider.configured) return 'API Key configured';
		return 'Not configured';
	}

	function isActive(provider: ProviderStatus): boolean {
		return provider.configured || provider.has_oauth;
	}

	type OAuthBrowserResult = {
		type?: string;
		provider?: string;
		success?: boolean;
		error?: string | null;
		issued_at?: number;
	};

	function handleOAuthMessage(event: MessageEvent) {
		if (event.origin !== window.location.origin) return;
		void completeOAuthFromBrowser(event.data);
	}

	function handleOAuthStorage(event: StorageEvent) {
		if (event.key !== 'krusty:oauth-result' || !event.newValue) return;
		try {
			void completeOAuthFromBrowser(JSON.parse(event.newValue));
		} catch {
			// Ignore invalid payloads
		}
	}

	async function consumeStoredOAuthResult() {
		const raw = localStorage.getItem('krusty:oauth-result');
		if (!raw) return;
		try {
			await completeOAuthFromBrowser(JSON.parse(raw));
		} catch {
			localStorage.removeItem('krusty:oauth-result');
		}
	}

	async function completeOAuthFromBrowser(raw: unknown) {
		const payload = parseOAuthBrowserResult(raw);
		if (!payload) return;

		localStorage.removeItem('krusty:oauth-result');

		if (payload.success) {
			await finishOAuthSuccess(payload.provider);
			return;
		}

		stopPolling();
		if (oauthLoading === payload.provider) {
			error = payload.error || 'Sign-in failed before Krusty received the token';
		}
		oauthLoading = null;
		oauthDialog = null;
		pasteCodeProvider = null;
		pasteCodeInput = '';
	}

	function parseOAuthBrowserResult(raw: unknown): Required<OAuthBrowserResult> | null {
		if (!raw || typeof raw !== 'object') return null;
		const payload = raw as OAuthBrowserResult;
		if (payload.type !== 'krusty-oauth-complete') return null;
		if (!payload.provider || typeof payload.provider !== 'string') return null;
		if (typeof payload.success !== 'boolean') return null;

		return {
			type: payload.type,
			provider: payload.provider,
			success: payload.success,
			error: typeof payload.error === 'string' ? payload.error : null,
			issued_at: typeof payload.issued_at === 'number' ? payload.issued_at : Date.now()
		};
	}

	async function finishOAuthSuccess(providerId: string) {
		stopPolling();
		oauthLoading = null;
		oauthDialog = null;
		pasteCodeProvider = null;
		pasteCodeInput = '';
		await loadProviders();
	}
</script>

<!-- OAuth dialog -->
{#if oauthDialog}
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
		<div class="w-full max-w-xl rounded-2xl border border-border bg-background shadow-2xl">
			<div class="flex items-center justify-between border-b border-border px-4 py-3">
				<div>
					<h3 class="font-semibold">
						{#if oauthDialog.kind === 'device'}
							OpenAI sign in
						{:else if oauthDialog.kind === 'browser_callback'}
							Complete OpenAI sign in
						{:else}
							Complete sign in
						{/if}
					</h3>
					<p class="text-xs text-muted-foreground">
						Authentication continues in your browser, not inside the PWA.
					</p>
				</div>
				<div class="flex items-center gap-2">
					<button
						onclick={() => openOAuthInNewTab()}
						class="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
						title="Open in new tab"
					>
						<ExternalLink class="h-3.5 w-3.5" />
						Open browser
					</button>
					<button onclick={closeOAuthDialog} class="rounded-lg p-2 hover:bg-muted">
						<X class="h-5 w-5" />
					</button>
				</div>
			</div>

			<div class="space-y-4 p-4">
				{#if oauthDialog.kind === 'browser_callback'}
					<p class="text-sm text-muted-foreground">
						Finish the OpenAI sign-in in your browser. When the callback lands back on Krusty,
						this screen should refresh automatically.
					</p>

					<div class="flex items-center justify-center gap-2 rounded-xl border border-border bg-muted/30 p-3 text-sm text-muted-foreground">
						<Loader2 class="h-4 w-4 animate-spin" />
						Waiting for browser sign-in to return to Krusty...
					</div>
				{:else if oauthDialog.kind === 'device'}
					<p class="text-sm text-muted-foreground">
						The browser page should ask you to confirm a device code. Enter this code if it is not pre-filled.
					</p>

					<div class="rounded-xl border border-border bg-card p-4 text-center">
						<div class="text-xs uppercase tracking-[0.2em] text-muted-foreground">User Code</div>
						<div class="mt-2 font-mono text-3xl font-semibold tracking-[0.3em]">
							{oauthDialog.deviceCode.user_code}
						</div>
					</div>

					<div class="rounded-xl border border-border bg-muted/40 p-3 text-xs text-muted-foreground">
						<div class="font-medium text-foreground">Verification URL</div>
						<div class="mt-1 break-all font-mono">{oauthDialog.deviceCode.verification_uri}</div>
					</div>

					<div class="flex items-center justify-center gap-2 rounded-xl border border-border bg-muted/30 p-3 text-sm text-muted-foreground">
						<Loader2 class="h-4 w-4 animate-spin" />
						Waiting for OpenAI authentication to complete...
					</div>
				{:else}
					<p class="text-sm text-muted-foreground">
						Finish the Anthropic sign-in in your browser, then paste the authorization code here.
					</p>

					<input
						bind:this={pasteCodeInputEl}
						type="text"
						bind:value={pasteCodeInput}
						placeholder="Paste authorization code..."
						class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm font-mono"
					/>

					<div class="flex gap-2">
						<button
							onclick={cancelPasteCode}
							class="flex-1 rounded-lg border border-border px-3 py-2 text-sm hover:bg-muted"
						>
							Cancel
						</button>
						<button
							onclick={submitPasteCode}
							disabled={!pasteCodeInput.trim() || oauthLoading === pasteCodeProvider}
							class="flex-1 rounded-lg bg-primary px-3 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
						>
							{#if oauthLoading === pasteCodeProvider}
								<Loader2 class="mx-auto h-4 w-4 animate-spin" />
							{:else}
								Submit Code
							{/if}
						</button>
					</div>
				{/if}
			</div>
		</div>
	</div>
{/if}

<div class="flex h-full flex-col">
	<!-- Header -->
	<div class="flex items-center gap-3 border-b border-border p-4">
		<button onclick={onBack} class="rounded-lg p-2 hover:bg-muted">
			<ArrowLeft class="h-5 w-5" />
		</button>
		<h2 class="text-lg font-semibold">AI Providers</h2>
	</div>

	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-4">
		{#if loading}
			<div class="flex items-center justify-center py-8">
				<Loader2 class="h-6 w-6 animate-spin text-muted-foreground" />
			</div>
		{:else if error}
			<div class="mb-4 rounded-lg bg-destructive/10 p-4 text-sm text-destructive">
				{error}
				<button onclick={() => (error = null)} class="ml-2 underline">dismiss</button>
			</div>
		{/if}

		{#if !loading}
			<p class="mb-4 text-sm text-muted-foreground">
				Configure API keys or sign in with OAuth to access AI providers.
			</p>

			<div class="space-y-3">
				{#each providers as provider}
					<div class="rounded-xl border border-border bg-card p-4">
						<!-- Provider header row -->
						<div class="flex items-center justify-between">
							<div class="flex items-center gap-3">
								<div
									class="flex h-8 w-8 items-center justify-center rounded-full {isActive(provider)
										? 'bg-green-500/20 text-green-500'
										: 'bg-muted text-muted-foreground'}"
								>
									{#if isActive(provider)}
										<Check class="h-4 w-4" />
									{:else}
										<X class="h-4 w-4" />
									{/if}
								</div>
								<div>
									<div class="font-medium">{provider.name}</div>
									<div class="text-xs text-muted-foreground">
										{statusText(provider)}
									</div>
								</div>
							</div>

							{#if editingProvider !== provider.id && pasteCodeProvider !== provider.id}
								<div class="flex gap-2">
									<!-- OAuth buttons -->
									{#if provider.supports_oauth}
										{#if provider.has_oauth}
											<button
												onclick={() => revokeOAuth(provider.id)}
												class="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm text-destructive hover:bg-destructive/10"
											>
												<LogOut class="h-3.5 w-3.5" />
												Sign out
											</button>
										{:else}
											<button
												onclick={() => startOAuth(provider.id)}
												disabled={oauthLoading === provider.id}
												class="flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
											>
												{#if oauthLoading === provider.id}
													<Loader2 class="h-3.5 w-3.5 animate-spin" />
													Signing in...
												{:else}
													<LogIn class="h-3.5 w-3.5" />
													Sign in
												{/if}
											</button>
										{/if}
									{/if}

									<!-- API key buttons -->
									{#if provider.configured}
										<button
											onclick={() => removeApiKey(provider.id)}
											class="rounded-lg px-3 py-1.5 text-sm text-destructive hover:bg-destructive/10"
										>
											Remove Key
										</button>
									{/if}
									<button
										onclick={() => startEditing(provider.id)}
										class="rounded-lg bg-muted px-3 py-1.5 text-sm hover:bg-accent"
									>
										{provider.configured ? 'Update Key' : 'Add Key'}
									</button>
								</div>
							{/if}
						</div>

						<!-- API key editing -->
						{#if editingProvider === provider.id}
							<div class="mt-4 space-y-3">
								<div class="relative">
									<input
										bind:this={apiKeyInputEl}
										type={showApiKey ? 'text' : 'password'}
										bind:value={apiKeyInput}
										placeholder="Enter API key..."
										class="w-full rounded-lg border border-border bg-background px-3 py-2 pr-10 text-sm"
									/>
									<button
										onclick={() => (showApiKey = !showApiKey)}
										class="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground hover:text-foreground"
									>
										{#if showApiKey}
											<EyeOff class="h-4 w-4" />
										{:else}
											<Eye class="h-4 w-4" />
										{/if}
									</button>
								</div>
								<div class="flex gap-2">
									<button
										onclick={cancelEditing}
										class="flex-1 rounded-lg border border-border px-3 py-2 text-sm hover:bg-muted"
									>
										Cancel
									</button>
									<button
										onclick={() => saveApiKey(provider.id)}
										disabled={!apiKeyInput.trim() || saving}
										class="flex-1 rounded-lg bg-primary px-3 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
									>
										{#if saving}
											<Loader2 class="mx-auto h-4 w-4 animate-spin" />
										{:else}
											Save
										{/if}
									</button>
								</div>
							</div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>
