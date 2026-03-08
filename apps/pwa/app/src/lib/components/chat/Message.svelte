<script lang="ts">


	import Copy from 'lucide-svelte/icons/copy';
	import Check from 'lucide-svelte/icons/check';
	import Clock from 'lucide-svelte/icons/clock';
	import { marked } from 'marked';
	import ToolWidget from './ToolWidget.svelte';
	import ToolApprovalWidget from './ToolApprovalWidget.svelte';
	import ThinkingBlock from './ThinkingBlock.svelte';
	import AskUserQuestionWidget from './AskUserQuestionWidget.svelte';
	import PlanConfirmWidget from './PlanConfirmWidget.svelte';
	import type { ChatMessage } from '$stores/session';

	interface Props {
		message: ChatMessage;
		isStreaming?: boolean;
	}

	let { message, isStreaming = false }: Props = $props();
	let copied = $state(false);

	const isUser = $derived(message.role === 'user');

	// Configure marked for code blocks
	marked.setOptions({
		gfm: true,
		breaks: true
	});

	function escapeHtml(str: string): string {
		return str
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;')
			.replace(/'/g, '&#039;');
	}

	// Custom renderer for code blocks
	const renderer = new marked.Renderer();
	renderer.code = ({ text, lang }) => {
		const language = escapeHtml(lang || 'text');
		const escapedText = escapeHtml(text);
		return `<div class="code-block" data-lang="${language}">
			<div class="code-header">
				<span class="code-lang">${language}</span>
				<button class="copy-btn" onclick="navigator.clipboard.writeText(this.closest('.code-block').querySelector('code').textContent)">
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
				</button>
			</div>
			<pre><code class="language-${language}">${escapedText}</code></pre>
		</div>`;
	};

	marked.use({ renderer });

	const renderedContent = $derived(
		message.content ? marked.parse(message.content) : ''
	);

	function copyMessage() {
		navigator.clipboard.writeText(message.content);
		copied = true;
		setTimeout(() => copied = false, 2000);
	}
</script>

{#if isUser}
	<!-- User message -->
	<div class="message-container group flex justify-end">
		<div class="flex min-w-0 max-w-[85%] flex-col gap-2">
			{#if message.isQueued}
				<div class="flex items-center gap-1.5 justify-end">
					<Clock class="h-3 w-3 text-amber-400" />
					<span class="text-xs font-medium text-amber-400">Queued</span>
				</div>
			{/if}
			<div class="message-bubble rounded-2xl px-4 py-3 rounded-tr-sm
				{message.isQueued ? 'bg-amber-500/20 text-amber-200' : 'bg-blue-500/20 text-blue-100 border border-blue-500/30'}">
				<div class="prose prose-sm max-w-none prose-invert">
					{@html renderedContent}
				</div>
			</div>
		</div>
	</div>
{:else}
	<!-- Assistant message - render as timeline of separate blocks -->
	<div class="assistant-timeline space-y-3">
		<!-- 1. Thinking block (if present) -->
		{#if message.thinking}
			<div class="timeline-entry">
				<ThinkingBlock content={message.thinking} isStreaming={isStreaming && !message.content && (!message.toolCalls || message.toolCalls.length === 0)} />
			</div>
		{/if}

		<!-- 2. Regular tool calls (NOT PlanConfirm or AskUserQuestion) -->
		{#if message.toolCalls && message.toolCalls.length > 0}
			{#each message.toolCalls.filter(tc => tc.name !== 'PlanConfirm' && tc.name !== 'AskUserQuestion' && tc.name !== 'enter_plan_mode' && tc.name !== 'set_work_mode' && tc.name !== 'task_start' && tc.name !== 'task_complete' && tc.name !== 'add_subtask' && tc.name !== 'set_dependency') as toolCall}
				<div class="timeline-entry">
					{#if toolCall.status === 'awaiting_approval'}
						<ToolApprovalWidget {toolCall} />
					{:else}
						<ToolWidget {toolCall} />
					{/if}
				</div>
			{/each}
		{/if}

		<!-- 3. Text content (the plan or response) -->
		{#if message.content}
			<div class="timeline-entry group flex gap-3">
				<div class="flex min-w-0 max-w-[85%] flex-col gap-2">
					<div class="relative">
						<div class="message-bubble rounded-2xl px-4 py-2 text-foreground">
							<div class="prose prose-sm max-w-none prose-neutral dark:prose-invert">
								{@html renderedContent}
							</div>
						</div>
						<button
							onclick={copyMessage}
							class="absolute -right-8 top-2 opacity-0 group-hover:opacity-100 transition-opacity
								p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
							title="Copy message"
						>
							{#if copied}
								<Check class="h-4 w-4 text-green-500" />
							{:else}
								<Copy class="h-4 w-4" />
							{/if}
						</button>
					</div>
				</div>
			</div>
		{/if}

		<!-- 4. Interactive widgets LAST (AskUserQuestion, PlanConfirm) -->
		{#if message.toolCalls && message.toolCalls.length > 0}
			{#each message.toolCalls.filter(tc => tc.name === 'AskUserQuestion') as toolCall}
				<div class="timeline-entry">
					<AskUserQuestionWidget {toolCall} />
				</div>
			{/each}
			{#each message.toolCalls.filter(tc => tc.name === 'PlanConfirm') as toolCall}
				<div class="timeline-entry">
					<PlanConfirmWidget
						{toolCall}
						planTitle={String(toolCall.arguments?.title ?? 'Implementation Plan')}
						taskCount={Number(toolCall.arguments?.task_count ?? 0)}
					/>
				</div>
			{/each}
		{/if}
	</div>
{/if}

<style>
	.timeline-entry {
		display: block;
		min-width: 0;
		overflow: hidden;
	}

	:global(.assistant-timeline) {
		min-width: 0;
		overflow: hidden;
	}

	:global(.message-bubble .prose) {
		font-size: 0.9rem;
		line-height: 1.6;
	}

	:global(.message-bubble .prose p) {
		margin: 0.5em 0;
	}

	:global(.message-bubble .prose p:first-child) {
		margin-top: 0;
	}

	:global(.message-bubble .prose p:last-child) {
		margin-bottom: 0;
	}

	:global(.message-bubble .prose code:not(pre code)) {
		background: hsl(var(--muted));
		padding: 0.15em 0.4em;
		border-radius: 0.25em;
		font-size: 0.85em;
	}

	:global(.message-bubble .code-block) {
		margin: 0.75em 0;
		border-radius: 0.5rem;
		overflow: hidden;
		background: hsl(var(--background));
		border: 1px solid hsl(var(--border));
	}

	:global(.message-bubble .code-header) {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 0.5rem 0.75rem;
		background: hsl(var(--muted));
		border-bottom: 1px solid hsl(var(--border));
		font-size: 0.75rem;
	}

	:global(.message-bubble .code-lang) {
		color: hsl(var(--muted-foreground));
		font-weight: 500;
	}

	:global(.message-bubble .copy-btn) {
		padding: 0.25rem;
		border-radius: 0.25rem;
		color: hsl(var(--muted-foreground));
		cursor: pointer;
		background: none;
		border: none;
	}

	:global(.message-bubble .copy-btn:hover) {
		background: hsl(var(--accent));
		color: hsl(var(--accent-foreground));
	}

	:global(.message-bubble pre) {
		margin: 0;
		padding: 0.75rem;
		overflow-x: auto;
		font-size: 0.8rem;
		line-height: 1.5;
	}

	:global(.message-bubble pre code) {
		background: none;
		padding: 0;
		font-size: inherit;
	}

	:global(.message-bubble ul, .message-bubble ol) {
		margin: 0.5em 0;
		padding-left: 1.5em;
	}

	:global(.message-bubble li) {
		margin: 0.25em 0;
	}

	:global(.message-bubble blockquote) {
		margin: 0.5em 0;
		padding-left: 1em;
		border-left: 3px solid hsl(var(--border));
		color: hsl(var(--muted-foreground));
	}

	:global(.message-bubble a) {
		color: hsl(var(--primary));
		text-decoration: underline;
	}

	:global(.message-bubble h1, .message-bubble h2, .message-bubble h3) {
		margin: 1em 0 0.5em;
		font-weight: 600;
	}

	:global(.message-bubble h1) { font-size: 1.25em; }
	:global(.message-bubble h2) { font-size: 1.1em; }
	:global(.message-bubble h3) { font-size: 1em; }
</style>
