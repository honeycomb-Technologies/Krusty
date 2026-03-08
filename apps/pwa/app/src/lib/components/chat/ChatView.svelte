<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import Send from 'lucide-svelte/icons/send';
	import Clock from 'lucide-svelte/icons/clock';
	import StopCircle from 'lucide-svelte/icons/stop-circle';
	import Paperclip from 'lucide-svelte/icons/paperclip';
	import Shield from 'lucide-svelte/icons/shield';
	import Zap from 'lucide-svelte/icons/zap';
	import Cpu from 'lucide-svelte/icons/cpu';
	import Brain from 'lucide-svelte/icons/brain';
	import Hammer from 'lucide-svelte/icons/hammer';
	import FileText from 'lucide-svelte/icons/file-text';

	import Mic from 'lucide-svelte/icons/mic';
	import X from 'lucide-svelte/icons/x';
	import Check from 'lucide-svelte/icons/check';
	import Message from './Message.svelte';
	import AsciiTitle from './AsciiTitle.svelte';
	import { sessionStore, sendMessage, stopGeneration, togglePermissionMode, toggleThinking, setMode, thinkingLevelLabel, type Attachment, type SessionMode } from '$stores/session';

	// Web Speech API type declarations (for browsers that support it)
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	type SpeechRecognitionType = any;

	interface Props {
		currentModel: string;
	}

	let { currentModel }: Props = $props();

	let inputValue = $state('');
	let inputElement = $state<HTMLTextAreaElement>(undefined!);
	let messagesContainer = $state<HTMLDivElement>(undefined!);
	let fileInput = $state<HTMLInputElement>(undefined!);
	let attachedFiles = $state<File[]>([]);
	
	// AI Controls expanded state
	let showAiControls = $state(false);

	// Voice transcription state
	let isTranscribing = $state(false);
	let transcribedText = $state('');

	// Check for Web Speech API support (with type assertion)
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	const isSpeechSupported = typeof window !== 'undefined' && 
		('SpeechRecognition' in window || 'webkitSpeechRecognition' in window);
	
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	function getSpeechRecognition(): any {
		if (typeof window === 'undefined') return null;
		return (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;
	}

	function toggleAiControls() {
		showAiControls = !showAiControls;
	}

	function closeAiControls() {
		showAiControls = false;
	}

	function handleModelClick() {
		// Emit event to parent to open model selector
		const event = new CustomEvent('openmodel');
		window.dispatchEvent(event);
		// Don't close menu - let user continue selecting options
	}

	function handleThinkClick() {
		toggleThinking();
		// Don't close menu - let user cycle through levels
	}

	function handleModeClick() {
		const newMode: SessionMode = $sessionStore.mode === 'build' ? 'plan' : 'build';
		setMode(newMode);
		// Don't close menu - let user continue selecting options
	}

	function handlePermClick() {
		togglePermissionMode();
		// Don't close menu - let user continue selecting options
	}

	function formatModelLabel(modelId: string): string {
		if (!modelId) return 'Default';
		const compact = modelId.includes('/') ? modelId.split('/').pop() || modelId : modelId;
		return compact.length > 16 ? `${compact.slice(0, 13)}...` : compact;
	}

	function startTranscription() {
		if (!isSpeechSupported || isTranscribing) return;

		isTranscribing = true;
		transcribedText = '';
		
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		const SpeechRecognition = getSpeechRecognition();
		if (!SpeechRecognition) {
			isTranscribing = false;
			return;
		}

		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		const recognition: any = new SpeechRecognition();
		recognition.continuous = true;
		recognition.interimResults = true;
		recognition.lang = 'en-US';

		recognition.onresult = (event: SpeechRecognitionType) => {
			let interimTranscript = '';
			let finalTranscript = '';

			for (let i = event.resultIndex; i < event.results.length; i++) {
				const transcript = event.results[i][0].transcript;
				if (event.results[i].isFinal) {
					finalTranscript += transcript + ' ';
				} else {
					interimTranscript += transcript;
				}
			}

			// Update input with transcribed text
			if (finalTranscript) {
				transcribedText += finalTranscript;
				inputValue = transcribedText;
				autoResize();
			} else if (interimTranscript) {
				// Show interim results
				inputValue = transcribedText + interimTranscript;
				autoResize();
			}
		};

		recognition.onerror = (event: SpeechRecognitionType) => {
			console.error('Speech recognition error:', event.error);
			stopTranscription();
		};

		recognition.onend = () => {
			// Only stop if user manually ended or there was an error
			// Don't auto-restart
		};

		// Store reference for stopping
		(window as unknown as { _speechRecognition: SpeechRecognitionType })._speechRecognition = recognition;

		try {
			recognition.start();
		} catch (e) {
			console.error('Failed to start speech recognition:', e);
			isTranscribing = false;
		}
	}

	function stopTranscription() {
		if (typeof window === 'undefined') {
			isTranscribing = false;
			return;
		}

		const recognition = (window as unknown as { _speechRecognition?: SpeechRecognitionType })._speechRecognition;
		if (recognition) {
			try {
				recognition.stop();
			} catch (e) {
				// Ignore errors when stopping
			}
			(window as unknown as { _speechRecognition?: SpeechRecognitionType })._speechRecognition = undefined;
		}
		isTranscribing = false;
	}

	function handleSubmit() {
		// If transcribing, stop first
		if (isTranscribing) {
			stopTranscription();
			return;
		}

		const hasText = inputValue.trim().length > 0;
		const hasContent = hasText || attachedFiles.length > 0;

		// While streaming, empty submit acts as stop.
		if ($sessionStore.isStreaming && !hasContent) {
			stopGeneration();
			return;
		}

		// Empty input on idle starts voice transcription (if supported).
		if (!hasContent) {
			if (isSpeechSupported) {
				startTranscription();
			}
			return;
		}

		// Allow sending with attachments or text (but at least one required)
		// Convert files to attachments
		const attachments: Attachment[] = attachedFiles.map(file => ({
			file,
			type: file.type.startsWith('image/') ? 'image' : 'file'
		}));

		sendMessage(inputValue.trim(), attachments);
		inputValue = '';
		attachedFiles = [];
		if (inputElement) {
			inputElement.value = '';
			inputElement.style.height = '';
		}
	}

	function handleFileSelect(e: Event) {
		const input = e.target as HTMLInputElement;
		if (input.files) {
			attachedFiles = [...attachedFiles, ...Array.from(input.files)];
		}
		input.value = '';
	}

	function removeFile(index: number) {
		attachedFiles = attachedFiles.filter((_, i) => i !== index);
	}

	function handleKeyDown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			handleSubmit();
		}
	}

	function autoResize() {
		if (inputElement) {
			// If empty, clear height to use min-h from CSS/Tailwind
			if (!inputElement.value) {
				inputElement.style.height = '';
				return;
			}
			// Otherwise expand to content
			inputElement.style.height = 'auto';
			inputElement.style.height = Math.min(inputElement.scrollHeight, 200) + 'px';
		}
	}

	$effect(() => {
		// Auto-scroll on new messages
		if ($sessionStore.messages.length && messagesContainer) {
			messagesContainer.scrollTop = messagesContainer.scrollHeight;
		}
	});

	onMount(() => {
		inputElement?.focus();
	});

	onDestroy(() => {
		stopTranscription();
	});
</script>

<div class="flex h-full min-h-0 flex-col" style:padding-bottom="var(--keyboard-height)">
	<!-- Error display -->
	{#if $sessionStore.error}
		<div class="mx-4 mt-2 rounded-lg bg-destructive/20 px-4 py-2 text-sm text-destructive">
			{$sessionStore.error}
		</div>
	{/if}

	<!-- Messages area -->
	<div
		bind:this={messagesContainer}
		class="messages-scroll min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-4"
	>
		{#if !$sessionStore.sessionId && $sessionStore.messages.length === 0}
			<!-- Welcome state - animated ASCII title -->
			<div class="flex h-full flex-col items-center justify-center"> 
				<AsciiTitle />
			</div>
		{:else if $sessionStore.messages.length === 0}
			<!-- Active session but no messages yet -->
		{:else}
			<div class="mx-auto max-w-3xl space-y-4"> 
				{#each $sessionStore.messages as message, i (i)}
					<Message {message} isStreaming={$sessionStore.isStreaming && i === $sessionStore.messages.length - 1} />
				{/each}
			</div>
		{/if}
	</div>

	<!-- Input area - only show when session is active -->
	{#if $sessionStore.sessionId}
		<div class="shrink-0 px-4 pb-2">
			<!-- Hidden file input - accepts all file types including images -->
			<input
				bind:this={fileInput}
				type="file"
				accept="image/*,.pdf,.txt,.md,.json,.js,.ts,.html,.css,.py,.rs,.go,.java,.c,.cpp,.h,.sh,.yaml,.yml,.toml,.xml,.csv"
				multiple
				class="hidden"
				onchange={handleFileSelect}
			/>

			<!-- Attached files preview -->
			{#if attachedFiles.length > 0}
				<div class="mx-auto mb-2 flex max-w-3xl flex-wrap gap-2"> 
					{#each attachedFiles as file, i}
						<div class="flex items-center gap-1 rounded-lg bg-muted px-2 py-1 text-xs">
							<span class="max-w-[150px] truncate">{file.name}</span>
							<button
								onclick={() => removeFile(i)}
								class="ml-1 text-muted-foreground hover:text-foreground"
							>×</button>
						</div>
					{/each}
				</div>
			{/if}

			<div class="mx-auto max-w-3xl">
				<div class="flex items-end gap-2 rounded-xl border border-border/50 bg-card/60 backdrop-blur-sm p-2">
					<!-- AI Controls -->
					<div class="relative">
						<button
							onclick={showAiControls ? closeAiControls : toggleAiControls}
							class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors
								{showAiControls 
										? 'bg-destructive text-destructive-foreground' 
										: 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
							title={showAiControls ? "Close menu" : "AI Controls"}
						>
							{#if showAiControls}
								<X class="h-4 w-4" />
							{:else}
								<img src="/icon.svg" alt="AI Controls" class="h-8 w-8 rounded" />
							{/if}
						</button>

						<!-- Expanded AI Controls Popover -->
						{#if showAiControls}
							<div class="absolute bottom-full left-0 mb-2 w-48 rounded-lg border border-border bg-card p-2 shadow-lg z-50 flex flex-col gap-1">
								<!-- Model - shows current model name -->
								<button
									onclick={handleModelClick}
									class="flex items-center justify-between rounded-md px-3 py-2 text-sm hover:bg-muted"
									title="Select model"
								>
									<div class="flex items-center gap-2">
										<Cpu class="h-4 w-4 text-muted-foreground" />
										<span>Model</span>
									</div>
									<span class="text-xs text-muted-foreground" title={currentModel}>
										{formatModelLabel(currentModel)}
									</span>
								</button>
								
								<!-- Think - cycles through thinking levels -->
								<button
									onclick={handleThinkClick}
									class="flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors hover:bg-muted
										{$sessionStore.thinkingEnabled ? 'bg-purple-500/20 text-purple-400' : ''}"
									title="Click to cycle thinking level"
								>
									<Brain class="h-4 w-4 {$sessionStore.thinkingEnabled ? 'text-purple-400' : 'text-muted-foreground'}" />
									<span>Think</span>
									{#if $sessionStore.thinkingEnabled}
										<span class="ml-auto text-xs uppercase text-purple-400">{thinkingLevelLabel($sessionStore.thinkingLevel)}</span>
									{/if}
								</button>
								
								<!-- Build/Plan -->
								<button
									onclick={handleModeClick}
									class="flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors hover:bg-muted
										{$sessionStore.mode === 'build' ? 'bg-orange-500/20 text-orange-400' : 'bg-green-500/20 text-green-400'}"
									title="Toggle mode"
								>
									{#if $sessionStore.mode === 'build'}
										<Hammer class="h-4 w-4 text-orange-400" />
										<span>Build</span>
									{:else}
										<FileText class="h-4 w-4 text-green-400" />
										<span>Plan</span>
									{/if}
								</button>
								
								<!-- Permission -->
								<button
									onclick={handlePermClick}
									class="flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors hover:bg-muted
										{$sessionStore.permissionMode === 'supervised' ? 'bg-amber-500/20 text-amber-400' : 'bg-green-500/20 text-green-400'}"
									title="Toggle permission mode"
								>
									{#if $sessionStore.permissionMode === 'supervised'}
										<Shield class="h-4 w-4 text-amber-400" />
										<span>Supervised</span>
									{:else}
										<Zap class="h-4 w-4 text-green-400" />
										<span>Auto</span>
									{/if}
								</button>
							</div>
						{/if}
					</div>

					<!-- Attachment button -->
					<button
						onclick={() => fileInput.click()}
						class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground
							transition-colors hover:bg-muted hover:text-foreground"
						title="Attach file or image"
					>
						<Paperclip class="h-4 w-4" />
					</button>

					<!-- Text input -->
					<textarea
						bind:this={inputElement}
						bind:value={inputValue}
						onkeydown={handleKeyDown}
						oninput={autoResize}
						placeholder={isTranscribing ? 'Listening...' : ($sessionStore.isStreaming ? 'Queue a message...' : 'Message Krusty...')}
						rows={1}
						enterkeyhint="send"
						class="max-h-[200px] min-h-[36px] flex-1 resize-none bg-transparent py-2 text-sm
							placeholder:text-muted-foreground focus:outline-none"
					></textarea>

					<!-- Combined Send/Queue/Stop/Voice button -->
					{#if isTranscribing}
						<!-- Transcription active - show check/stop button (green, reassuring) -->
						<button
							onclick={handleSubmit}
							class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-green-500 text-white hover:bg-green-600 transition-colors"
							title="Click to stop recording"
						>
							<Check class="h-4 w-4" />
						</button>
					{:else}
						<!-- Unified send/queue/stop/voice button -->
						{@const hasDraftContent = inputValue.trim().length > 0 || attachedFiles.length > 0}
						{@const isQueueing = $sessionStore.isStreaming && hasDraftContent}
						{@const isStopping = $sessionStore.isStreaming && !hasDraftContent}
						{@const canStartVoice = isSpeechSupported && !hasDraftContent && !$sessionStore.isStreaming}
						<button
							onclick={handleSubmit}
							disabled={!$sessionStore.isStreaming && !hasDraftContent && !isSpeechSupported}
							class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors
								{isQueueing
										? 'bg-amber-500 text-white hover:bg-amber-600'
										: isStopping
												? 'bg-destructive text-white hover:bg-destructive/90'
												: canStartVoice
														? 'bg-blue-500 text-white hover:bg-blue-600'
														: 'bg-primary text-primary-foreground hover:bg-primary/90'
								}
								{!$sessionStore.isStreaming && !hasDraftContent && !isSpeechSupported ? 'disabled:cursor-not-allowed disabled:opacity-50' : ''}"
							title={isQueueing
								? 'Queue message'
								: isStopping
										? 'Stop generation'
										: canStartVoice
												? 'Start voice transcription'
												: 'Send message'
							}
						>
							{#if isQueueing}
								<Clock class="h-4 w-4" />
							{:else if isStopping}
								<StopCircle class="h-4 w-4" />
							{:else if canStartVoice}
								<Mic class="h-4 w-4" />
							{:else}
								<Send class="h-4 w-4" />
							{/if}
						</button>
					{/if}
				</div>
			</div>
		</div>
	{/if}

</div>

<style>
	.messages-scroll {
		/* Prevent over-scroll bounce */
		overscroll-behavior: contain;

		/* GPU acceleration for smooth scrolling */
		will-change: scroll-position;
		transform: translateZ(0);

		/* Contain layout for performance */
		contain: strict;

		/* Smooth native scrolling on touch devices */
		-webkit-overflow-scrolling: touch;

		/* Hide scrollbar but keep functionality */
		scrollbar-width: thin;
		scrollbar-color: hsl(var(--muted-foreground) / 0.3) transparent;
	}

	.messages-scroll::-webkit-scrollbar {
		width: 6px;
	}

	.messages-scroll::-webkit-scrollbar-track {
		background: transparent;
	}

	.messages-scroll::-webkit-scrollbar-thumb {
		background: hsl(var(--muted-foreground) / 0.3);
		border-radius: 3px;
	}

	.messages-scroll::-webkit-scrollbar-thumb:hover {
		background: hsl(var(--muted-foreground) / 0.5);
	}
</style>
