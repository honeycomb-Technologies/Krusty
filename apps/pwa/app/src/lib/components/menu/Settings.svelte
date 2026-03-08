<script lang="ts">
	import { onMount } from 'svelte';
	import { ArrowLeft, Bell, Loader2, Server, TestTube2, Monitor } from 'lucide-svelte';
	import { apiClient, type PreviewSettings, type PushStatusResponse } from '$lib/api/client';
	import {
		getCurrentPushState,
		isPushSupported,
		reconcilePushSubscription,
		subscribeToPush,
		type PushState,
		unsubscribeFromPush
	} from '$lib/push';

	interface Props {
		onBack: () => void;
	}

	let { onBack }: Props = $props();

	let notifications = $state(false);
	let initializing = $state(true);
	let subscribing = $state(false);
	let pushError = $state('');
	let pushSupported = $state(false);
	let pushPermission = $state<NotificationPermission | 'unsupported'>('unsupported');
	let pushEndpoint = $state<string | null>(null);
	let pushStatus = $state<PushStatusResponse | null>(null);
	let statusError = $state('');
	let testingPush = $state(false);
	let testMessage = $state('');
	let serverUrl = $state('http://localhost:3000');
	let previewSettings = $state<PreviewSettings | null>(null);
	let previewLoading = $state(true);
	let previewSaving = $state(false);
	let previewError = $state('');

	onMount(() => {
		void Promise.all([initializePushSettings(), loadPreviewSettings()]);
	});

	function applyPushState(state: PushState) {
		notifications = state.subscribed;
		pushSupported = state.supported;
		pushPermission = state.permission;
		pushEndpoint = state.endpoint;
	}

	async function refreshPushStatus() {
		statusError = '';
		try {
			pushStatus = await apiClient.getPushStatus();
		} catch (e) {
			pushStatus = null;
			statusError = e instanceof Error ? e.message : 'Failed to load push status';
		}
	}

	async function initializePushSettings() {
		initializing = true;
		pushError = '';
		try {
			applyPushState(await reconcilePushSubscription());
		} catch (e) {
			pushError = e instanceof Error ? e.message : 'Failed to initialize push notifications';
			try {
				applyPushState(await getCurrentPushState());
			} catch {
				pushSupported = isPushSupported();
			}
		}
		await refreshPushStatus();
		initializing = false;
	}

	async function toggleNotifications() {
		if (subscribing || initializing) return;
		subscribing = true;
		pushError = '';
		testMessage = '';

		try {
			if (notifications) {
				await unsubscribeFromPush();
			} else {
				const ok = await subscribeToPush();
				if (!ok) {
					pushError =
						Notification.permission === 'denied'
							? 'Permission denied in browser settings'
							: 'Permission not granted';
				}
			}
			applyPushState(await getCurrentPushState());
			await refreshPushStatus();
		} catch (e) {
			pushError = e instanceof Error ? e.message : 'Failed';
			try {
				applyPushState(await getCurrentPushState());
			} catch {
				// ignore
			}
		} finally {
			subscribing = false;
		}
	}

	async function sendTestNotification() {
		if (testingPush) return;
		testingPush = true;
		testMessage = '';
		pushError = '';

		try {
			const result = await apiClient.sendPushTest();
			if (result.sent > 0) {
				testMessage = `Sent to ${result.sent}/${result.attempted} subscription${result.sent === 1 ? '' : 's'}.`;
			} else if (result.attempted === 0) {
				testMessage = 'No active subscriptions found on the server.';
			} else {
				testMessage = `No successful deliveries (${result.failed} failed, ${result.stale_removed} stale removed).`;
			}
			await refreshPushStatus();
		} catch (e) {
			testMessage = e instanceof Error ? e.message : 'Failed to send test notification';
		} finally {
			testingPush = false;
		}
	}

	async function loadPreviewSettings() {
		previewLoading = true;
		previewError = '';
		try {
			previewSettings = await apiClient.getPreviewSettings();
		} catch (e) {
			previewError = e instanceof Error ? e.message : 'Failed to load preview settings';
		} finally {
			previewLoading = false;
		}
	}

	async function togglePreviewEnabled() {
		if (!previewSettings || previewSaving) return;
		previewSaving = true;
		previewError = '';
		try {
			previewSettings = await apiClient.updatePreviewSettings({
				enabled: !previewSettings.enabled
			});
		} catch (e) {
			previewError = e instanceof Error ? e.message : 'Failed to update preview settings';
		} finally {
			previewSaving = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	<!-- Header -->
	<div class="flex items-center gap-3 border-b border-border px-4 py-3">
		<button onclick={onBack} class="rounded-lg p-1 hover:bg-muted">
			<ArrowLeft class="h-5 w-5" />
		</button>
		<h2 class="flex-1 text-lg font-semibold">Settings</h2>
	</div>

	<!-- Settings content -->
	<div class="flex-1 overflow-y-auto p-4">
		<!-- Notifications -->
		<section class="mb-6">
			<h3 class="mb-3 text-sm font-medium text-muted-foreground">Notifications</h3>
			<div class="space-y-2">
				<div class="flex items-center justify-between rounded-xl bg-card p-4">
					<div class="flex items-center gap-3">
						<Bell class="h-5 w-5 text-muted-foreground" />
						<div>
							<div class="font-medium">Push Notifications</div>
							<div class="text-sm text-muted-foreground">
								{#if initializing}
									Loading state...
								{:else if subscribing}
									Updating subscription...
								{:else if pushError}
									{pushError}
								{:else if !pushSupported}
									Not supported in this browser
								{:else if pushPermission === 'denied'}
									Permission denied in browser settings
								{:else if notifications}
									Subscribed on this device
								{:else}
									Not subscribed
								{/if}
							</div>
							<div class="text-xs text-muted-foreground">
								Permission: {pushPermission}
								{#if pushEndpoint}
									· Synced endpoint
								{/if}
							</div>
						</div>
					</div>
					<button
						onclick={toggleNotifications}
						disabled={initializing || subscribing || !pushSupported}
						aria-label="Toggle push notifications"
						class="relative h-6 w-11 rounded-full transition-colors
							{notifications ? 'bg-primary' : 'bg-muted'}
							{initializing || subscribing || !pushSupported ? 'opacity-50' : ''}"
					>
						<span
							class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform
								{notifications ? 'left-[22px]' : 'left-0.5'}"
						></span>
					</button>
				</div>

				<div class="rounded-xl bg-card p-4">
					<div class="mb-3 flex items-center gap-3">
						<Server class="h-5 w-5 text-muted-foreground" />
						<div>
							<div class="font-medium">Delivery Diagnostics</div>
							<div class="text-sm text-muted-foreground">
								Server health and recent push delivery outcomes
							</div>
						</div>
					</div>

					{#if statusError}
						<div class="mb-3 text-sm text-destructive">{statusError}</div>
					{:else if !pushStatus}
						<div class="mb-3 flex items-center gap-2 text-sm text-muted-foreground">
							<Loader2 class="h-4 w-4 animate-spin" />
							Loading push status...
						</div>
					{:else}
						<div class="mb-3 grid grid-cols-2 gap-x-3 gap-y-1 text-sm">
							<div class="text-muted-foreground">Configured</div>
							<div>{pushStatus.push_configured ? 'Yes' : 'No'}</div>
							<div class="text-muted-foreground">Subscriptions</div>
							<div>{pushStatus.subscription_count}</div>
							<div class="text-muted-foreground">Last Attempt</div>
							<div>{pushStatus.last_attempt_at ?? 'Never'}</div>
							<div class="text-muted-foreground">Last Success</div>
							<div>{pushStatus.last_success_at ?? 'Never'}</div>
							<div class="text-muted-foreground">Failures (24h)</div>
							<div>{pushStatus.recent_failures_24h}</div>
						</div>
						{#if pushStatus.last_failure_reason}
							<div class="mb-3 text-xs text-destructive">
								Last failure: {pushStatus.last_failure_reason}
							</div>
						{/if}
					{/if}

					<button
						onclick={sendTestNotification}
						disabled={testingPush || !notifications}
						class="flex w-full items-center justify-center gap-2 rounded-lg border border-input px-3 py-2 text-sm transition-colors hover:bg-muted disabled:opacity-50"
					>
						{#if testingPush}
							<Loader2 class="h-4 w-4 animate-spin" />
						{:else}
							<TestTube2 class="h-4 w-4" />
						{/if}
						Send Test Notification
					</button>
					{#if testMessage}
						<div class="mt-2 text-xs text-muted-foreground">{testMessage}</div>
					{/if}
				</div>
			</div>
		</section>

		<!-- Preview & Port Forwarding -->
		<section class="mb-6">
			<h3 class="mb-3 text-sm font-medium text-muted-foreground">Preview & Port Forwarding</h3>
			<div class="space-y-2">
				<div class="flex items-center justify-between rounded-xl bg-card p-4">
					<div class="flex items-center gap-3">
						<Monitor class="h-5 w-5 text-muted-foreground" />
						<div>
							<div class="font-medium">Port Forwarding</div>
							<div class="text-sm text-muted-foreground">
								{#if previewLoading}
									Loading...
								{:else if !previewSettings}
									Unavailable
								{:else if previewSettings.enabled}
									Auto-detecting local servers
								{:else}
									Disabled
								{/if}
							</div>
						</div>
					</div>
					{#if previewSettings && !previewLoading}
						<button
							onclick={togglePreviewEnabled}
							disabled={previewSaving}
							aria-label="Toggle port forwarding"
							class="relative h-6 w-11 rounded-full transition-colors
								{previewSettings.enabled ? 'bg-primary' : 'bg-muted'}
								{previewSaving ? 'opacity-50' : ''}"
						>
							<span
								class="absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform
									{previewSettings.enabled ? 'left-[22px]' : 'left-0.5'}"
							></span>
						</button>
					{/if}
				</div>
				{#if previewError}
					<div class="text-sm text-destructive">{previewError}</div>
				{/if}
			</div>
		</section>

		<!-- Connection -->
		<section>
			<h3 class="mb-3 text-sm font-medium text-muted-foreground">Connection</h3>
			<div class="space-y-2">
				<div class="rounded-xl bg-card p-4">
					<div class="mb-3 flex items-center gap-3">
						<Server class="h-5 w-5 text-muted-foreground" />
						<div>
							<div class="font-medium">Server URL</div>
							<div class="text-sm text-muted-foreground">Krusty server address</div>
						</div>
					</div>
					<input
						type="url"
						bind:value={serverUrl}
						class="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm
							placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring"
					/>
				</div>
			</div>
		</section>
	</div>
</div>
