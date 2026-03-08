<script lang="ts">
	import Brain from 'lucide-svelte/icons/brain';
	import ChevronDown from 'lucide-svelte/icons/chevron-down';
	import ChevronUp from 'lucide-svelte/icons/chevron-up';

	interface Props {
		content: string;
		isStreaming?: boolean;
	}

	let { content, isStreaming = false }: Props = $props();
	let isExpanded = $state(false);
</script>

<div class="w-full">
	<button
		onclick={() => (isExpanded = !isExpanded)}
		class="flex w-full items-center gap-2 px-1 py-1.5 text-left text-sm text-purple-300/80
			transition-colors hover:text-purple-300"
	>
		<Brain class="h-4 w-4 shrink-0" />
		{#if isStreaming}
			<div class="thinking-dots flex gap-1">
				<span></span>
				<span></span>
				<span></span>
			</div>
			<span class="flex-1 text-xs font-medium">Thinking...</span>
		{:else}
			<span class="flex-1 text-xs font-medium">Thought process</span>
		{/if}
		{#if isExpanded}
			<ChevronUp class="h-3.5 w-3.5" />
		{:else}
			<ChevronDown class="h-3.5 w-3.5" />
		{/if}
	</button>

	{#if isExpanded && content}
		<div class="px-1 pt-1 pb-2">
			<pre class="whitespace-pre-wrap font-mono text-xs text-muted-foreground leading-relaxed">{content}</pre>
		</div>
	{/if}
</div>

<style>
	.thinking-dots span {
		width: 5px;
		height: 5px;
		background: currentColor;
		border-radius: 50%;
		animation: thinking 1.4s ease-in-out infinite;
	}

	.thinking-dots span:nth-child(1) { animation-delay: 0s; }
	.thinking-dots span:nth-child(2) { animation-delay: 0.2s; }
	.thinking-dots span:nth-child(3) { animation-delay: 0.4s; }

	@keyframes thinking {
		0%, 80%, 100% {
			opacity: 0.3;
			transform: scale(0.8);
		}
		40% {
			opacity: 1;
			transform: scale(1);
		}
	}
</style>
