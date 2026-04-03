<script lang="ts">
	import Compass from 'lucide-svelte/icons/compass';
	import Hammer from 'lucide-svelte/icons/hammer';
	import CheckIcon from 'lucide-svelte/icons/check';
	import X from 'lucide-svelte/icons/x';
	import Loader2 from 'lucide-svelte/icons/loader-2';
	import ChevronDown from 'lucide-svelte/icons/chevron-down';
	import ChevronUp from 'lucide-svelte/icons/chevron-up';
	import ThinkingBlock from './ThinkingBlock.svelte';
	import type { ToolCall } from '$stores/session';

	interface Props {
		toolCall: ToolCall;
		delegatedThinking?: string;
		hasParentReview?: boolean;
	}

	let { toolCall, delegatedThinking, hasParentReview = false }: Props = $props();
	let expanded = $state(false);

	const delegated = $derived(toolCall.delegated);
	const isExplore = $derived(toolCall.name === 'explore');
	const headerIcon = $derived(isExplore ? Compass : Hammer);
	const isSingleAgent = $derived(isExplore && (!delegated?.agents || delegated.agents.length <= 1));
	const headerTitle = $derived(isExplore ? 'Consortium' : 'Kraken');
	const headerLabel = $derived(isExplore ? (isSingleAgent ? 'agent' : 'agents') : 'builders');
	const Icon = $derived(headerIcon);

	function formatDuration(ms?: number): string | null {
		if (!ms || ms <= 0) return null;
		const seconds = Math.floor(ms / 1000);
		if (seconds >= 60) {
			const minutes = Math.floor(seconds / 60);
			return `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
		}
		return `${seconds}s`;
	}

	function formatAgentStatus(status: 'pending' | 'running' | 'complete' | 'failed'): string {
		switch (status) {
			case 'pending':
				return 'pending';
			case 'running':
				return 'running';
			case 'complete':
				return 'complete';
			case 'failed':
				return 'failed';
		}
	}

	function formatOutcomeReason(reason?: string): string | null {
		switch (reason) {
			case 'invalid_target':
				return 'Target directory was invalid or outside the project';
			case 'misread_tool_output':
				return 'Agent misread successful tool output';
			case 'missing_structured_report':
				return 'Agent finished without a valid structured exploration report';
			case 'provider_failure':
				return 'Provider failed during delegated run';
			case 'no_usable_evidence':
				return 'No usable evidence was gathered';
			case 'mixed':
				return 'Delegated run failed for mixed reasons';
			default:
				return null;
		}
	}

	function formatRunStage(
		stage?: 'created' | 'running' | 'synthesizing' | 'complete' | 'degraded' | 'failed' | 'cancelled'
	): string | null {
		switch (stage) {
			case 'created':
				return 'created';
			case 'running':
				return 'running';
			case 'synthesizing':
				return 'synthesizing';
			case 'complete':
				return 'complete';
			case 'degraded':
				return 'degraded';
			case 'failed':
				return 'failed';
			case 'cancelled':
				return 'cancelled';
			default:
				return null;
		}
	}

	const detailSummary = $derived.by(() => {
		const artifact = delegated;
		if (!artifact) return [];
		const items: string[] = [];
		if (toolCall.status === 'running' && artifact.totalTurns != null) items.push(`${artifact.totalTurns} turns`);
		if (artifact.linesAdded != null || artifact.linesRemoved != null) {
			items.push(`+${artifact.linesAdded || 0} -${artifact.linesRemoved || 0}`);
		}
		return items;
	});

	const headerStatus = $derived(toolCall.status === 'partial' ? 'partial' : toolCall.status);
	const showInlineSummary = $derived(toolCall.status === 'running' || !hasParentReview);
</script>

<div class="delegated-widget">
	<div class="delegated-card" class:delegated-active={toolCall.status === 'running'}>
		<div class="delegated-header" class:delegated-build={!isExplore}>
			<div class="flex items-center gap-2">
				<Icon class="h-3.5 w-3.5" />
				<span class="delegated-title">{headerTitle}</span>
				{#if headerStatus === 'running'}
					<Loader2 class="h-3 w-3 animate-spin opacity-80" />
				{:else if headerStatus === 'success'}
					<CheckIcon class="h-3 w-3" />
				{:else if headerStatus === 'partial'}
					<div class="partial-icon">~</div>
				{:else if toolCall.status === 'error'}
					<X class="h-3 w-3 text-red-300" />
				{/if}
			</div>
			<div class="flex items-center gap-2 text-[10px] opacity-75">
				{#if !isSingleAgent && (delegated?.totalTargets || delegated?.agentCount || (delegated?.agents && delegated.agents.length > 1))}
					<span>{delegated?.totalTargets || delegated?.agentCount || delegated?.agents.length} total {headerLabel}</span>
				{/if}
				{#if (delegated?.activeTargets ?? 0) > 0}
					<span>{delegated?.activeTargets} active</span>
				{/if}
				{#if (delegated?.pendingTargets ?? 0) > 0}
					<span>{delegated?.pendingTargets} pending</span>
				{/if}
				{#if formatRunStage(delegated?.stage)}
					<span>{formatRunStage(delegated?.stage)}</span>
				{/if}
				{#if !isSingleAgent && (delegated?.usableAgents != null || delegated?.degradedAgents != null || delegated?.failedAgents != null)}
					<span>
						{delegated?.usableAgents ?? delegated?.successfulAgents ?? 0} usable
						{#if (delegated?.degradedAgents ?? 0) > 0}
							• {delegated?.degradedAgents ?? 0} degraded
						{/if}
						• {delegated?.failedAgents || 0} failed
					</span>
				{/if}
				{#if detailSummary.length > 0}
					<span>{detailSummary.join(' • ')}</span>
				{/if}
				{#if formatDuration(delegated?.totalDurationMs)}
					<span>{formatDuration(delegated?.totalDurationMs)}</span>
				{/if}
			</div>
		</div>

		{#if delegated?.message && showInlineSummary}
			<div class="delegated-summary">{delegated?.message}</div>
		{/if}

		{#if delegatedThinking && toolCall.status === 'running'}
			<div class="delegated-thinking">
				<ThinkingBlock content={delegatedThinking} isStreaming={toolCall.status === 'running'} />
			</div>
		{/if}

		{#if delegated?.investigationSummary && showInlineSummary}
			<div class="delegated-summary">{delegated.investigationSummary}</div>
		{/if}

		{#if delegated?.coverageGapNotice && showInlineSummary}
			<div class="delegated-summary delegated-warning">{delegated.coverageGapNotice}</div>
		{/if}

		{#if delegated?.outcome === 'failed' && (delegated?.filesExaminedCount ?? delegated?.filesExamined.length ?? 0) === 0}
			<div class="delegated-summary delegated-warning">
				{formatOutcomeReason(delegated?.outcomeReason) || 'No usable evidence was gathered. The investigation should stop and ask for narrower scope instead of continuing broad reads.'}
			</div>
		{/if}

		{#if delegated?.agents && delegated.agents.length > 0}
			<div class="delegated-agents">
				{#each delegated.agents as agent}
					<div class="delegated-agent-row">
						<div class="delegated-agent-status status-{agent.status}">
							{#if agent.status === 'pending'}
								<div class="pending-icon">…</div>
							{:else if agent.status === 'running'}
								<Loader2 class="h-3 w-3 animate-spin" />
							{:else if agent.status === 'complete'}
								<CheckIcon class="h-3 w-3" />
							{:else}
								<X class="h-3 w-3" />
							{/if}
						</div>
						<div class="delegated-agent-body">
							<div class="delegated-agent-meta">
								<span class="delegated-agent-name">{agent.name}</span>
								<span class="delegated-agent-badge">{formatAgentStatus(agent.status)}</span>
								{#if formatOutcomeReason(agent.outcomeReason)}
									<span class="delegated-agent-badge">{formatOutcomeReason(agent.outcomeReason)}</span>
								{/if}
								{#if agent.toolCount > 0}
									<span class="delegated-agent-badge">{agent.toolCount} tools</span>
								{/if}
								{#if agent.tokens > 0}
									<span class="delegated-agent-badge">{agent.tokens} tok</span>
								{/if}
								{#if agent.linesAdded > 0 || agent.linesRemoved > 0}
									<span class="delegated-agent-badge">+{agent.linesAdded} -{agent.linesRemoved}</span>
								{/if}
							</div>
							{#if agent.currentAction || agent.completionSummary}
								<div class="delegated-agent-text">{agent.status === 'running' ? agent.currentAction : (agent.completionSummary || agent.currentAction)}</div>
							{/if}
						</div>
					</div>
				{/each}
			</div>
		{/if}

		{#if (delegated?.filesExamined && delegated.filesExamined.length > 0) || (delegated?.errors && delegated.errors.length > 0)}
			<button class="delegated-details-toggle" onclick={() => (expanded = !expanded)}>
				<span>Details</span>
				{#if expanded}
					<ChevronUp class="h-3 w-3" />
				{:else}
					<ChevronDown class="h-3 w-3" />
				{/if}
			</button>
		{/if}

		{#if expanded}
			<div class="delegated-details">
				{#if delegated?.filesExamined && delegated.filesExamined.length > 0}
					<div class="delegated-section">
					<div class="delegated-section-title">Evidence examined</div>
						{#each delegated.filesExamined as file}
							<div class="delegated-list-item">{file}</div>
						{/each}
					</div>
				{/if}

				{#if delegated?.errors && delegated.errors.length > 0}
					<div class="delegated-section">
						<div class="delegated-section-title delegated-error-title">Errors</div>
						{#each delegated.errors as error}
							<div class="delegated-error-item">{error}</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}

		{#if toolCall.arguments?.prompt && (toolCall.status === 'running' || !hasParentReview)}
			<div class="delegated-prompt">{toolCall.arguments.prompt}</div>
		{/if}
	</div>
</div>

<style>
	.delegated-widget {
		min-width: 0;
		overflow: hidden;
	}

	.delegated-card {
		border-radius: 0.75rem;
		border: 1px solid #27272a;
		background: #0a0a0a;
		overflow: hidden;
	}

	.delegated-card.delegated-active {
		border-color: hsl(var(--thinking) / 0.4);
	}

	.delegated-thinking {
		padding: 0.25rem 0.75rem 0.5rem;
		border-top: 1px solid hsl(var(--border) / 0.4);
		border-bottom: 1px solid hsl(var(--border) / 0.4);
	}

	.pending-icon {
		font-size: 0.9rem;
		line-height: 1;
		opacity: 0.7;
	}

	.delegated-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.625rem 0.75rem;
		border-bottom: 1px solid #27272a;
		color: #67e8f9;
		background: linear-gradient(135deg, rgba(34,211,238,0.08), rgba(59,130,246,0.06));
		font-family: 'JetBrains Mono', monospace;
	}

	.delegated-header.delegated-build {
		color: #fdba74;
		background: linear-gradient(135deg, rgba(249,115,22,0.08), rgba(251,191,36,0.06));
	}

	.delegated-title {
		font-size: 0.75rem;
		font-weight: 600;
		letter-spacing: 0.05em;
	}

	.delegated-summary,
	.delegated-prompt {
		padding: 0.5rem 0.75rem;
		font-size: 0.7rem;
		color: #a1a1aa;
		font-family: 'JetBrains Mono', monospace;
		border-bottom: 1px solid #18181b;
	}

	.delegated-warning {
		color: #fbbf24;
		background: rgba(245, 158, 11, 0.08);
	}

	.delegated-prompt {
		color: #71717a;
		border-bottom: 0;
		border-top: 1px solid #18181b;
	}

	.delegated-agents {
		padding: 0.375rem 0;
	}

	.delegated-agent-row {
		display: flex;
		gap: 0.5rem;
		padding: 0.4rem 0.75rem;
	}

	.delegated-agent-status {
		width: 1rem;
		display: flex;
		align-items: center;
		justify-content: center;
		color: #93c5fd;
	}

	.delegated-agent-status.status-complete {
		color: #4ade80;
	}

	.delegated-agent-status.status-failed {
		color: #f87171;
	}

	.partial-icon {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 0.75rem;
		height: 0.75rem;
		border-radius: 999px;
		font-size: 0.62rem;
		line-height: 1;
		background: rgba(251, 191, 36, 0.18);
		color: #fcd34d;
	}

	.delegated-agent-body {
		min-width: 0;
		flex: 1;
	}

	.delegated-agent-meta {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem;
		align-items: center;
		font-family: 'JetBrains Mono', monospace;
	}

	.delegated-agent-name {
		font-size: 0.72rem;
		color: #e4e4e7;
		font-weight: 600;
	}

	.delegated-agent-badge {
		font-size: 0.62rem;
		color: #a1a1aa;
		border: 1px solid #27272a;
		border-radius: 999px;
		padding: 0.08rem 0.38rem;
	}

	.delegated-agent-text {
		margin-top: 0.15rem;
		font-size: 0.68rem;
		color: #71717a;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		font-family: 'JetBrains Mono', monospace;
	}

	.delegated-details-toggle {
		width: 100%;
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.45rem 0.75rem;
		border-top: 1px solid #18181b;
		font-size: 0.68rem;
		color: #a1a1aa;
		background: transparent;
	}

	.delegated-details {
		padding: 0.5rem 0.75rem 0.75rem;
		border-top: 1px solid #18181b;
	}

	.delegated-section + .delegated-section {
		margin-top: 0.6rem;
	}

	.delegated-section-title {
		margin-bottom: 0.35rem;
		font-size: 0.68rem;
		color: #d4d4d8;
		font-weight: 600;
	}

	.delegated-error-title {
		color: #fda4af;
	}

	.delegated-list-item,
	.delegated-error-item {
		font-size: 0.66rem;
		color: #71717a;
		font-family: 'JetBrains Mono', monospace;
		word-break: break-word;
	}

	.delegated-error-item {
		color: #fca5a5;
	}
</style>
