<script lang="ts">
	import Terminal from 'lucide-svelte/icons/terminal';
	import FileText from 'lucide-svelte/icons/file-text';
	import FilePlus from 'lucide-svelte/icons/file-plus';
	import FolderTree from 'lucide-svelte/icons/folder-tree';
	import FolderOpen from 'lucide-svelte/icons/folder-open';
	import Search from 'lucide-svelte/icons/search';
	import Edit3 from 'lucide-svelte/icons/edit-3';
	import FileDiffIcon from 'lucide-svelte/icons/file-diff';
	import Layers from 'lucide-svelte/icons/layers';
	import CheckIcon from 'lucide-svelte/icons/check';
	import X from 'lucide-svelte/icons/x';
	import ChevronDown from 'lucide-svelte/icons/chevron-down';
	import ChevronUp from 'lucide-svelte/icons/chevron-up';
	import Loader2 from 'lucide-svelte/icons/loader-2';
	import Compass from 'lucide-svelte/icons/compass';
	import Hammer from 'lucide-svelte/icons/hammer';
	import { FileDiff, getFiletypeFromFileName } from '@pierre/diffs';
	import type { ToolCall } from '$stores/session';

	interface Props {
		toolCall: ToolCall;
	}

	let { toolCall }: Props = $props();
	let isExpanded = $state(false);
	let elapsedSeconds = $state(0);
	let startTime: number | null = $state(null);
	let timerInterval: ReturnType<typeof setInterval> | null = null;
	let bashOutputEl: HTMLPreElement | undefined = $state();

	// Track elapsed time while tool is running
	$effect(() => {
		if (toolCall.status === 'running') {
			if (!startTime) startTime = Date.now();
			timerInterval = setInterval(() => {
				elapsedSeconds = Math.floor((Date.now() - startTime!) / 1000);
			}, 1000);
		} else {
			if (timerInterval) {
				clearInterval(timerInterval);
				timerInterval = null;
			}
		}

		return () => {
			if (timerInterval) clearInterval(timerInterval);
		};
	});

	// Auto-scroll bash output while streaming
	$effect(() => {
		if (isBashTool && toolCall.output && bashOutputEl) {
			bashOutputEl.scrollTop = bashOutputEl.scrollHeight;
		}
	});

	const iconMap: Record<string, typeof Terminal> = {
		bash: Terminal,
		read: FileText,
		write: FilePlus,
		edit: Edit3,
		multiedit: Layers,
		glob: FolderTree,
		grep: Search,
		list: FolderOpen,
		apply_patch: FileDiffIcon,
		explore: Compass,
		build: Hammer,
	};

	const Icon = $derived(iconMap[toolCall.name] || Terminal);

	const statusStyles = $derived({
		running: 'border-blue-500/50 bg-blue-500/10',
		success: 'border-green-500/50 bg-green-500/10',
		error: 'border-red-500/50 bg-red-500/10',
		pending: 'border-border bg-muted/50',
		awaiting_approval: 'border-amber-500/50 bg-amber-500/10'
	}[toolCall.status] || 'border-border bg-muted/50');

	// Detect tool types
	const isEditTool = $derived(
		toolCall.arguments?.old_string != null &&
		toolCall.arguments?.new_string != null
	);

	const isBashTool = $derived(toolCall.name === 'bash');
	const isReadTool = $derived(toolCall.name === 'read');
	const isWriteTool = $derived(toolCall.name === 'write' && !isEditTool);
	const isGlobTool = $derived(toolCall.name === 'glob');
	const isGrepTool = $derived(toolCall.name === 'grep');
	const isListTool = $derived(toolCall.name === 'list');
	const isApplyPatchTool = $derived(toolCall.name === 'apply_patch');
	const isMultiEditTool = $derived(toolCall.name === 'multiedit');
	const isExploreTool = $derived(toolCall.name === 'explore');
	const isBuildTool = $derived(toolCall.name === 'build');

	const bashCommand = $derived(
		isBashTool ? String(toolCall.arguments?.command || '') : ''
	);

	function parseToolPayload(raw: string): Record<string, unknown> | null {
		try {
			const parsed = JSON.parse(raw);
			if (parsed && typeof parsed === 'object') {
				const payload = (parsed as { data?: unknown }).data;
				if (payload && typeof payload === 'object') return payload as Record<string, unknown>;
				return parsed as Record<string, unknown>;
			}
			return null;
		} catch {
			return null;
		}
	}

	function parseToolErrorText(raw: string): string {
		const payload = parseToolPayload(raw);
		if (!payload) return raw;

		const root = (() => {
			try {
				return JSON.parse(raw) as Record<string, unknown>;
			} catch {
				return null;
			}
		})();
		const error = root && typeof root.error === 'object' ? root.error as Record<string, unknown> : null;
		const errorMessage = error && typeof error.message === 'string' ? error.message : null;
		if (errorMessage) return errorMessage;

		const output = payload.output;
		if (typeof output === 'string' && output.length > 0) return output;
		return raw;
	}

	const bashOutput = $derived(() => {
		if (!isBashTool || !toolCall.output) return '';
		try {
			const parsed = JSON.parse(toolCall.output);
			return (
				parsed?.data?.output ??
				parsed?.output ??
				parsed?.error?.message ??
				toolCall.output
			);
		} catch {
			return toolCall.output;
		}
	});

	const toolFilePath = $derived(
		String(toolCall.arguments?.file_path || 'file')
	);

	const shortPath = $derived(() => {
		const parts = toolFilePath.split('/');
		return parts.length > 2 ? parts.slice(-2).join('/') : toolFilePath;
	});

	// Glob results: parse output as file list
	const globResults = $derived(() => {
		if (!isGlobTool || !toolCall.output) return [];
		const payload = parseToolPayload(toolCall.output);
		const matches = payload?.matches;
		if (Array.isArray(matches)) {
			return matches
				.filter((v): v is string => typeof v === 'string')
				.filter((l: string) => l.trim());
		}
		return toolCall.output.split('\n').filter((l: string) => l.trim());
	});

	// Grep results: parse output lines
	const grepResults = $derived(() => {
		if (!isGrepTool || !toolCall.output) return [];
		const payload = parseToolPayload(toolCall.output);
		const matches = payload?.matches;
		if (Array.isArray(matches)) {
			return matches
				.map((m) => {
					if (!m || typeof m !== 'object') return '';
					const record = m as Record<string, unknown>;
					const file = typeof record.file === 'string' ? record.file : '';
					const line = typeof record.line === 'string' ? record.line : '';
					const lineNumber = typeof record.line_number === 'number' ? record.line_number : null;
					if (!file) return line;
					return lineNumber != null ? `${file}:${lineNumber}: ${line}` : `${file}: ${line}`;
				})
				.filter((l: string) => l.trim());
		}
		const files = payload?.files;
		if (Array.isArray(files)) {
			return files.filter((v): v is string => typeof v === 'string');
		}
		const counts = payload?.counts;
		if (Array.isArray(counts)) {
			return counts
				.map((c) => {
					if (!c || typeof c !== 'object') return '';
					const record = c as Record<string, unknown>;
					const file = typeof record.file === 'string' ? record.file : '';
					const count = typeof record.count === 'number' ? record.count : null;
					return file && count != null ? `${file}: ${count}` : '';
				})
				.filter((l: string) => l.trim());
		}
		return toolCall.output.split('\n').filter((l: string) => l.trim());
	});

	// Explore/Build: parse agent progress from output
	interface AgentInfo {
		name: string;
		status: 'running' | 'complete' | 'failed';
		toolCount: number;
		tokens: string;
		action: string;
	}

	const agentInfos = $derived((): AgentInfo[] => {
		if (!isExploreTool && !isBuildTool) return [];
		if (!toolCall.output) return [];

		// Parse "## Agent: name" sections from output
		const agents: AgentInfo[] = [];
		const sections = toolCall.output.split(/^## Agent: /m);
		for (const section of sections) {
			if (!section.trim()) continue;
			const lines = section.split('\n');
			const name = lines[0]?.trim() || 'agent';
			agents.push({
				name: name.substring(0, 12),
				status: 'complete',
				toolCount: 0,
				tokens: '',
				action: lines.slice(1).join('\n').trim().substring(0, 100) || '',
			});
		}

		// If no parsed agents but still running, show a placeholder
		if (agents.length === 0 && toolCall.status === 'running') {
			const prompt = String(toolCall.arguments?.prompt || '');
			const components = (toolCall.arguments?.components as string[]) || (toolCall.arguments?.directories as string[]) || [];
			if (components.length > 0) {
				for (const comp of components) {
					const parts = comp.split('/');
					agents.push({
						name: parts[parts.length - 1]?.substring(0, 12) || 'agent',
						status: 'running',
						toolCount: 0,
						tokens: '',
						action: prompt.substring(0, 60),
					});
				}
			} else {
				agents.push({
					name: 'agent',
					status: 'running',
					toolCount: 0,
					tokens: '',
					action: prompt.substring(0, 60),
				});
			}
		}

		return agents;
	});

	// List results
	const listResults = $derived(() => {
		if (!isListTool || !toolCall.output) return [];
		const payload = parseToolPayload(toolCall.output);
		const output = typeof payload?.output === 'string' ? payload.output : toolCall.output;
		return output.split('\n').filter((l: string) => l.trim());
	});

	const toolErrorText = $derived(
		toolCall.output ? parseToolErrorText(toolCall.output) : ''
	);

	// Apply patch results
	const patchResults = $derived(() => {
		if (!isApplyPatchTool || !toolCall.output) return { modified: [], created: [], deleted: [], message: '' };
		try {
			const parsed = JSON.parse(toolCall.output);
			const payload = parsed.data || parsed;
			return {
				modified: payload.files_modified || [],
				created: payload.files_created || [],
				deleted: payload.files_deleted || [],
				message: payload.message || parsed?.error?.message || '',
			};
		} catch {
			return { modified: [], created: [], deleted: [], message: toolCall.output };
		}
	});

	// Diff containers
	let diffContainer: HTMLDivElement | undefined = $state();
	let writeDiffContainer: HTMLDivElement | undefined = $state();
	let multiEditDiffContainer: HTMLDivElement | undefined = $state();

	// Edit tool diff
	$effect(() => {
		if (!isEditTool || !diffContainer) return;

		const args = toolCall.arguments!;
		const fp = String(args.file_path || 'file');
		const lang = getFiletypeFromFileName(fp) ?? undefined;

		const instance = new FileDiff({
			theme: 'pierre-dark',
			themeType: 'dark',
			diffStyle: 'unified',
			overflow: 'wrap',
			disableFileHeader: true,
			disableLineNumbers: false,
			unsafeCSS: `
				pre { background: transparent !important; }
				code { background: transparent !important; }
			`,
		});

		instance.render({
			oldFile: { name: fp, contents: String(args.old_string), lang },
			newFile: { name: fp, contents: String(args.new_string), lang },
			containerWrapper: diffContainer,
		});

		return () => {
			instance.cleanUp();
		};
	});

	// Write tool diff
	$effect(() => {
		if (!isWriteTool || !writeDiffContainer) return;

		const args = toolCall.arguments!;
		const fp = String(args.file_path || 'file');
		const content = String(args.content || '');
		const lang = getFiletypeFromFileName(fp) ?? undefined;

		const instance = new FileDiff({
			theme: 'pierre-dark',
			themeType: 'dark',
			diffStyle: 'unified',
			overflow: 'wrap',
			disableFileHeader: true,
			disableLineNumbers: false,
			unsafeCSS: `
				pre { background: transparent !important; }
				code { background: transparent !important; }
			`,
		});

		instance.render({
			oldFile: { name: fp, contents: '', lang },
			newFile: { name: fp, contents: content, lang },
			containerWrapper: writeDiffContainer,
		});

		return () => {
			instance.cleanUp();
		};
	});

	// MultiEdit tool diff - combine all old_string -> new_string pairs
	$effect(() => {
		if (!isMultiEditTool || !multiEditDiffContainer) return;

		const args = toolCall.arguments!;
		const fp = String(args.file_path || 'file');
		const edits = (args.edits as Array<{ old_string: string; new_string: string }>) || [];
		const lang = getFiletypeFromFileName(fp) ?? undefined;

		const oldParts = edits.map((e) => String(e.old_string)).join('\n...\n');
		const newParts = edits.map((e) => String(e.new_string)).join('\n...\n');

		const instance = new FileDiff({
			theme: 'pierre-dark',
			themeType: 'dark',
			diffStyle: 'unified',
			overflow: 'wrap',
			disableFileHeader: true,
			disableLineNumbers: false,
			unsafeCSS: `
				pre { background: transparent !important; }
				code { background: transparent !important; }
			`,
		});

		instance.render({
			oldFile: { name: fp, contents: oldParts, lang },
			newFile: { name: fp, contents: newParts, lang },
			containerWrapper: multiEditDiffContainer,
		});

		return () => {
			instance.cleanUp();
		};
	});

	const description = $derived(() => {
		if (toolCall.description) return toolCall.description;
		if (!toolCall.arguments) return null;

		const args = toolCall.arguments;
		switch (toolCall.name) {
			case 'bash':
				return args.command as string;
			case 'read':
			case 'write':
			case 'edit':
			case 'multiedit':
				return args.file_path as string;
			case 'glob':
				return args.pattern as string;
			case 'grep':
				return `${args.pattern} in ${args.path || '.'}`;
			case 'list':
				return args.path as string;
			case 'apply_patch':
				return 'Applying patch';
			case 'explore':
				return args.prompt as string;
			case 'build':
				return args.prompt as string;
			default:
				return null;
		}
	});

	function formatTime(seconds: number): string {
		if (seconds >= 60) {
			const m = Math.floor(seconds / 60);
			const s = seconds % 60;
			return `${m}:${String(s).padStart(2, '0')}`;
		}
		return `${seconds}s`;
	}
</script>

{#if isEditTool}
	<!-- Edit tool: diff view -->
	<div class="edit-tool-widget">
		<div class="flex items-center gap-2 px-1 py-1.5">
			<Edit3 class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="flex-1 min-w-0 truncate text-xs font-mono {toolCall.status === 'running' ? 'text-blue-300' : 'text-muted-foreground'}">{shortPath()}</span>
			{#if toolCall.status === 'running'}
				<div class="editing-dots flex items-center gap-1">
					<span></span><span></span><span></span>
				</div>
				<span class="text-xs text-blue-400 tabular-nums">{elapsedSeconds}s</span>
			{:else if toolCall.status === 'success'}
				<CheckIcon class="h-3.5 w-3.5 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3.5 w-3.5 text-red-500" />
			{/if}
		</div>

		<div bind:this={diffContainer} class="diff-wrapper {toolCall.status === 'running' ? 'diff-active' : ''}"></div>

		{#if toolCall.status === 'error' && toolCall.output}
			<pre class="mt-2 rounded-lg bg-red-500/10 border border-red-500/30 p-2.5 font-mono text-xs whitespace-pre-wrap text-red-300">{toolErrorText}</pre>
		{/if}
	</div>

{:else if isBashTool}
	<!-- Bash tool: terminal view -->
	<div class="bash-widget">
		<div class="bash-terminal" class:bash-active={toolCall.status === 'running'}>
			<div class="bash-titlebar">
				<div class="flex items-center gap-1.5">
					<span class="dot dot-red"></span>
					<span class="dot dot-yellow"></span>
					<span class="dot dot-green"></span>
				</div>
				<span class="bash-title">bash</span>
				<div class="flex items-center gap-1.5">
					{#if toolCall.status === 'running'}
						<Loader2 class="h-3 w-3 animate-spin text-zinc-400" />
						<span class="text-[10px] text-zinc-500 tabular-nums">{elapsedSeconds}s</span>
					{:else if toolCall.status === 'success'}
						<CheckIcon class="h-3 w-3 text-green-500" />
					{:else if toolCall.status === 'error'}
						<X class="h-3 w-3 text-red-500" />
					{/if}
				</div>
			</div>

			<pre class="bash-body" bind:this={bashOutputEl}><span class="bash-prompt">$</span> <span class="bash-cmd">{bashCommand}</span>{#if bashOutput}
{bashOutput}{/if}</pre>
		</div>
	</div>

{:else if isReadTool}
	<!-- Read tool: minimal file label -->
	<div class="flex items-center gap-2 px-1 py-1.5">
		<FileText class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
		<span class="text-xs text-muted-foreground">read</span>
		<span class="flex-1 min-w-0 truncate text-xs font-mono {toolCall.status === 'running' ? 'text-blue-300' : 'text-muted-foreground'}">{shortPath()}</span>
		{#if toolCall.status === 'running'}
			<Loader2 class="h-3 w-3 animate-spin text-blue-400" />
		{:else if toolCall.status === 'success'}
			<CheckIcon class="h-3 w-3 text-green-500" />
		{:else if toolCall.status === 'error'}
			<X class="h-3 w-3 text-red-500" />
		{/if}
	</div>

{:else if isWriteTool}
	<!-- Write tool: diff view (new file) -->
	<div class="edit-tool-widget">
		<div class="flex items-center gap-2 px-1 py-1.5">
			<FilePlus class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="flex-1 min-w-0 truncate text-xs font-mono {toolCall.status === 'running' ? 'text-blue-300' : 'text-muted-foreground'}">{shortPath()}</span>
			{#if toolCall.status === 'running'}
				<div class="editing-dots flex items-center gap-1">
					<span></span><span></span><span></span>
				</div>
				<span class="text-xs text-blue-400 tabular-nums">{elapsedSeconds}s</span>
			{:else if toolCall.status === 'success'}
				<CheckIcon class="h-3.5 w-3.5 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3.5 w-3.5 text-red-500" />
			{/if}
		</div>

		<div bind:this={writeDiffContainer} class="diff-wrapper {toolCall.status === 'running' ? 'diff-active' : ''}"></div>

		{#if toolCall.status === 'error' && toolCall.output}
			<pre class="mt-2 rounded-lg bg-red-500/10 border border-red-500/30 p-2.5 font-mono text-xs whitespace-pre-wrap text-red-300">{toolErrorText}</pre>
		{/if}
	</div>

{:else if isGlobTool}
	<!-- Glob tool: minimal expandable file list -->
	<div class="min-w-0">
		<button onclick={() => (isExpanded = !isExpanded)} class="flex w-full items-center gap-2 px-1 py-1.5 text-left">
			<FolderTree class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="text-xs text-muted-foreground">glob</span>
			<span class="flex-1 min-w-0 truncate text-xs font-mono text-muted-foreground">{toolCall.arguments?.pattern}</span>
			{#if toolCall.status === 'running'}
				<Loader2 class="h-3 w-3 animate-spin text-blue-400" />
			{:else if toolCall.status === 'success'}
				{#if globResults().length > 0}
					<span class="text-[10px] text-muted-foreground tabular-nums">{globResults().length} files</span>
				{/if}
				<CheckIcon class="h-3 w-3 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3 w-3 text-red-500" />
			{/if}
			{#if isExpanded}
				<ChevronUp class="h-3 w-3 text-muted-foreground" />
			{:else}
				<ChevronDown class="h-3 w-3 text-muted-foreground" />
			{/if}
		</button>

		{#if isExpanded && globResults().length > 0}
			<div class="glob-results ml-6 border-l border-border/50 pl-3 py-1">
				{#each globResults() as file}
					<div class="flex items-center gap-1.5 py-0.5">
						<FileText class="h-3 w-3 shrink-0 text-muted-foreground/60" />
						<span class="text-xs font-mono text-muted-foreground truncate">{file}</span>
					</div>
				{/each}
			</div>
		{/if}
	</div>

{:else if isGrepTool}
	<!-- Grep tool: minimal expandable results -->
	<div class="min-w-0">
		<button onclick={() => (isExpanded = !isExpanded)} class="flex w-full items-center gap-2 px-1 py-1.5 text-left">
			<Search class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="text-xs text-muted-foreground">grep</span>
			<span class="flex-1 min-w-0 truncate text-xs font-mono text-muted-foreground">{toolCall.arguments?.pattern}</span>
			{#if toolCall.status === 'running'}
				<Loader2 class="h-3 w-3 animate-spin text-blue-400" />
			{:else if toolCall.status === 'success'}
				{#if grepResults().length > 0}
					<span class="text-[10px] text-muted-foreground tabular-nums">{grepResults().length} matches</span>
				{/if}
				<CheckIcon class="h-3 w-3 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3 w-3 text-red-500" />
			{/if}
			{#if isExpanded}
				<ChevronUp class="h-3 w-3 text-muted-foreground" />
			{:else}
				<ChevronDown class="h-3 w-3 text-muted-foreground" />
			{/if}
		</button>

		{#if isExpanded && grepResults().length > 0}
			<div class="grep-results ml-6 border-l border-border/50 pl-3 py-1 max-h-48 overflow-y-auto">
				{#each grepResults() as line}
					<div class="py-0.5">
						<span class="text-xs font-mono text-muted-foreground">{line}</span>
					</div>
				{/each}
			</div>
		{/if}
	</div>

{:else if isListTool}
	<!-- List tool: expandable directory listing -->
	<div class="min-w-0">
		<button onclick={() => (isExpanded = !isExpanded)} class="flex w-full items-center gap-2 px-1 py-1.5 text-left">
			<FolderOpen class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="text-xs text-muted-foreground">list</span>
			<span class="flex-1 min-w-0 truncate text-xs font-mono text-muted-foreground">{toolCall.arguments?.path}</span>
			{#if toolCall.status === 'running'}
				<Loader2 class="h-3 w-3 animate-spin text-blue-400" />
			{:else if toolCall.status === 'success'}
				{#if listResults().length > 0}
					<span class="text-[10px] text-muted-foreground tabular-nums">{listResults().length} entries</span>
				{/if}
				<CheckIcon class="h-3 w-3 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3 w-3 text-red-500" />
			{/if}
			{#if isExpanded}
				<ChevronUp class="h-3 w-3 text-muted-foreground" />
			{:else}
				<ChevronDown class="h-3 w-3 text-muted-foreground" />
			{/if}
		</button>

		{#if isExpanded && listResults().length > 0}
			<div class="glob-results ml-6 border-l border-border/50 pl-3 py-1 max-h-64 overflow-y-auto">
				{#each listResults() as entry}
					<div class="flex items-center gap-1.5 py-0.5">
						{#if entry.endsWith('/')}
							<FolderOpen class="h-3 w-3 shrink-0 text-blue-400/60" />
						{:else}
							<FileText class="h-3 w-3 shrink-0 text-muted-foreground/60" />
						{/if}
						<span class="text-xs font-mono text-muted-foreground truncate">{entry}</span>
					</div>
				{/each}
			</div>
		{/if}
	</div>

{:else if isApplyPatchTool}
	<!-- Apply Patch tool: file status list -->
	<div class="min-w-0">
		<button onclick={() => (isExpanded = !isExpanded)} class="flex w-full items-center gap-2 px-1 py-1.5 text-left">
			<FileDiffIcon class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="text-xs text-muted-foreground">patch</span>
			<span class="flex-1 min-w-0 truncate text-xs font-mono text-muted-foreground">{patchResults().message || 'Applying patch...'}</span>
			{#if toolCall.status === 'running'}
				<div class="editing-dots flex items-center gap-1">
					<span></span><span></span><span></span>
				</div>
				<span class="text-xs text-blue-400 tabular-nums">{elapsedSeconds}s</span>
			{:else if toolCall.status === 'success'}
				<CheckIcon class="h-3.5 w-3.5 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3.5 w-3.5 text-red-500" />
			{/if}
			{#if isExpanded}
				<ChevronUp class="h-3 w-3 text-muted-foreground" />
			{:else}
				<ChevronDown class="h-3 w-3 text-muted-foreground" />
			{/if}
		</button>

		{#if isExpanded && toolCall.status !== 'running'}
			<div class="ml-6 border-l border-border/50 pl-3 py-1">
				{#each patchResults().modified as file}
					<div class="flex items-center gap-1.5 py-0.5">
						<Edit3 class="h-3 w-3 shrink-0 text-yellow-400/70" />
						<span class="text-xs font-mono text-muted-foreground truncate">{file}</span>
					</div>
				{/each}
				{#each patchResults().created as file}
					<div class="flex items-center gap-1.5 py-0.5">
						<FilePlus class="h-3 w-3 shrink-0 text-green-400/70" />
						<span class="text-xs font-mono text-muted-foreground truncate">{file}</span>
					</div>
				{/each}
				{#each patchResults().deleted as file}
					<div class="flex items-center gap-1.5 py-0.5">
						<X class="h-3 w-3 shrink-0 text-red-400/70" />
						<span class="text-xs font-mono text-muted-foreground truncate">{file}</span>
					</div>
				{/each}
			</div>
		{/if}

		{#if toolCall.status === 'error' && toolCall.output}
			<pre class="mt-2 rounded-lg bg-red-500/10 border border-red-500/30 p-2.5 font-mono text-xs whitespace-pre-wrap text-red-300">{toolErrorText}</pre>
		{/if}
	</div>

{:else if isMultiEditTool}
	<!-- MultiEdit tool: diff view -->
	<div class="edit-tool-widget">
		<div class="flex items-center gap-2 px-1 py-1.5">
			<Layers class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
			<span class="text-xs text-muted-foreground">multiedit</span>
			<span class="flex-1 min-w-0 truncate text-xs font-mono {toolCall.status === 'running' ? 'text-blue-300' : 'text-muted-foreground'}">{shortPath()}</span>
			{#if toolCall.arguments?.edits}
				<span class="text-[10px] text-muted-foreground tabular-nums">{(toolCall.arguments.edits as unknown[]).length} edits</span>
			{/if}
			{#if toolCall.status === 'running'}
				<div class="editing-dots flex items-center gap-1">
					<span></span><span></span><span></span>
				</div>
				<span class="text-xs text-blue-400 tabular-nums">{elapsedSeconds}s</span>
			{:else if toolCall.status === 'success'}
				<CheckIcon class="h-3.5 w-3.5 text-green-500" />
			{:else if toolCall.status === 'error'}
				<X class="h-3.5 w-3.5 text-red-500" />
			{/if}
		</div>

		<div bind:this={multiEditDiffContainer} class="diff-wrapper {toolCall.status === 'running' ? 'diff-active' : ''}"></div>

		{#if toolCall.status === 'error' && toolCall.output}
			<pre class="mt-2 rounded-lg bg-red-500/10 border border-red-500/30 p-2.5 font-mono text-xs whitespace-pre-wrap text-red-300">{toolErrorText}</pre>
		{/if}
	</div>

{:else if isExploreTool}
	<!-- Explore tool: Consortium widget -->
	<div class="swarm-widget">
		<div class="swarm-card" class:swarm-active={toolCall.status === 'running'}>
			<div class="swarm-header swarm-explore">
				<div class="flex items-center gap-2">
					<Compass class="h-3.5 w-3.5" />
					<span class="swarm-name">Consortium</span>
					{#if toolCall.status === 'running'}
						<span class="pincer-anim"></span>
					{:else}
						<CheckIcon class="h-3 w-3" />
					{/if}
				</div>
				<div class="flex items-center gap-2">
					{#if agentInfos().length > 0}
						<span class="text-[10px] opacity-70">{agentInfos().length} agents</span>
					{/if}
					{#if toolCall.status === 'running'}
						<span class="text-[10px] opacity-70 tabular-nums">{formatTime(elapsedSeconds)}</span>
					{/if}
				</div>
			</div>

			{#if agentInfos().length > 0}
				<div class="swarm-agents">
					{#each agentInfos() as agent}
						<div class="swarm-agent-row">
							<div class="swarm-agent-status">
								{#if agent.status === 'running'}
									<span class="agent-spinner"></span>
								{:else if agent.status === 'complete'}
									<span class="text-green-500">✓</span>
								{:else}
									<span class="text-red-500">✗</span>
								{/if}
							</div>
							<span class="swarm-agent-name">{agent.name}</span>
							{#if agent.action}
								<span class="swarm-agent-action">{agent.action}</span>
							{/if}
						</div>
					{/each}
				</div>
			{/if}

			{#if toolCall.arguments?.prompt}
				<div class="swarm-prompt">
					<span class="truncate">{toolCall.arguments.prompt}</span>
				</div>
			{/if}
		</div>
	</div>

{:else if isBuildTool}
	<!-- Build tool: Kraken widget -->
	<div class="swarm-widget">
		<div class="swarm-card" class:swarm-active={toolCall.status === 'running'}>
			<div class="swarm-header swarm-build">
				<div class="flex items-center gap-2">
					<Hammer class="h-3.5 w-3.5" />
					<span class="swarm-name">Kraken</span>
					{#if toolCall.status === 'running'}
						<span class="wave-anim">▁▃▅▇▅▃▁</span>
					{:else}
						<CheckIcon class="h-3 w-3" />
					{/if}
				</div>
				<div class="flex items-center gap-2">
					{#if agentInfos().length > 0}
						<span class="text-[10px] opacity-70">{agentInfos().length} builders</span>
					{/if}
					{#if toolCall.status === 'running'}
						<span class="text-[10px] opacity-70 tabular-nums">{formatTime(elapsedSeconds)}</span>
					{/if}
				</div>
			</div>

			{#if agentInfos().length > 0}
				<div class="swarm-agents">
					{#each agentInfos() as agent}
						<div class="swarm-agent-row">
							<div class="swarm-agent-status">
								{#if agent.status === 'running'}
									<span class="agent-spinner"></span>
								{:else if agent.status === 'complete'}
									<span class="text-green-500">✓</span>
								{:else}
									<span class="text-red-500">✗</span>
								{/if}
							</div>
							<span class="swarm-agent-name">{agent.name}</span>
							{#if agent.action}
								<span class="swarm-agent-action">{agent.action}</span>
							{/if}
						</div>
					{/each}
				</div>
			{/if}

			{#if toolCall.arguments?.prompt}
				<div class="swarm-prompt">
					<span class="truncate">{toolCall.arguments.prompt}</span>
				</div>
			{/if}
		</div>
	</div>

{:else}
	<!-- Fallback: minimal label for unknown tools (processes, skill, etc.) -->
	<div class="flex items-center gap-2 px-1 py-1.5">
		<Icon class="h-3.5 w-3.5 {toolCall.status === 'running' ? 'text-blue-400' : 'text-muted-foreground'}" />
		<span class="text-xs text-muted-foreground">{toolCall.name}</span>
		{#if description()}
			<span class="flex-1 min-w-0 truncate text-xs font-mono text-muted-foreground">{description()}</span>
		{/if}
		{#if toolCall.status === 'running'}
			<Loader2 class="h-3 w-3 animate-spin text-blue-400" />
			<span class="text-xs text-muted-foreground tabular-nums">{elapsedSeconds}s</span>
		{:else if toolCall.status === 'success'}
			<CheckIcon class="h-3 w-3 text-green-500" />
		{:else if toolCall.status === 'error'}
			<X class="h-3 w-3 text-red-500" />
		{/if}
	</div>
{/if}

<style>
	/* === Edit / Write tool === */
	.edit-tool-widget {
		min-width: 0;
		overflow: hidden;
	}

	.diff-wrapper {
		border-radius: 0.5rem;
		overflow: hidden;
		border: 1px solid hsl(var(--border));
		transition: border-color 0.3s ease;
	}

	.diff-wrapper.diff-active {
		border-color: hsl(217 91% 60% / 0.5);
		animation: diff-pulse 2s ease-in-out infinite;
	}

	@keyframes diff-pulse {
		0%, 100% { border-color: hsl(217 91% 60% / 0.3); }
		50% { border-color: hsl(217 91% 60% / 0.6); }
	}

	.editing-dots span {
		width: 4px;
		height: 4px;
		background: hsl(217 91% 60%);
		border-radius: 50%;
		animation: editing-bounce 1.4s ease-in-out infinite;
	}

	.editing-dots span:nth-child(1) { animation-delay: 0s; }
	.editing-dots span:nth-child(2) { animation-delay: 0.2s; }
	.editing-dots span:nth-child(3) { animation-delay: 0.4s; }

	@keyframes editing-bounce {
		0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
		40% { opacity: 1; transform: scale(1); }
	}

	.diff-wrapper :global(diffs-container) {
		--diffs-font-family: 'JetBrains Mono', monospace;
		--diffs-font-size: 12px;
		--diffs-line-height: 18px;
		display: block;
	}

	/* === Bash terminal === */
	.bash-widget {
		min-width: 0;
		overflow: hidden;
	}

	.bash-terminal {
		border-radius: 0.5rem;
		overflow: hidden;
		border: 1px solid #27272a;
		background: #0a0a0a;
		transition: border-color 0.3s ease;
	}

	.bash-terminal.bash-active {
		border-color: hsl(217 91% 60% / 0.4);
	}

	.bash-titlebar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.4rem 0.75rem;
		background: #18181b;
		border-bottom: 1px solid #27272a;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
	}

	.dot-red { background: #ef4444; }
	.dot-yellow { background: #eab308; }
	.dot-green { background: #22c55e; }

	.bash-title {
		font-family: 'JetBrains Mono', monospace;
		font-size: 0.65rem;
		color: #52525b;
		letter-spacing: 0.05em;
	}

	.bash-body {
		margin: 0;
		padding: 0.75rem;
		font-family: 'JetBrains Mono', monospace;
		font-size: 12px;
		line-height: 1.6;
		color: #fafafa;
		white-space: pre-wrap;
		word-break: break-all;
		max-height: 300px;
		overflow-y: auto;
		scrollbar-width: thin;
		scrollbar-color: #3f3f46 transparent;
	}

	.bash-body::-webkit-scrollbar { width: 4px; }
	.bash-body::-webkit-scrollbar-track { background: transparent; }
	.bash-body::-webkit-scrollbar-thumb { background: #3f3f46; border-radius: 2px; }

	.bash-prompt { color: #22c55e; font-weight: 600; }
	.bash-cmd { color: #fafafa; }

	/* === Glob/Grep results === */
	.glob-results, .grep-results {
		scrollbar-width: thin;
		scrollbar-color: hsl(var(--muted-foreground) / 0.2) transparent;
	}

	/* === Swarm widgets (Explore/Build) === */
	.swarm-widget {
		min-width: 0;
		overflow: hidden;
	}

	.swarm-card {
		border-radius: 0.5rem;
		overflow: hidden;
		border: 1px solid #27272a;
		background: #0a0a0a;
		transition: border-color 0.3s ease;
	}

	.swarm-card.swarm-active {
		border-color: hsl(var(--thinking) / 0.4);
	}

	.swarm-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.5rem 0.75rem;
		border-bottom: 1px solid #27272a;
		font-size: 0.75rem;
		font-weight: 600;
		font-family: 'JetBrains Mono', monospace;
	}

	.swarm-header.swarm-explore {
		background: linear-gradient(135deg, #06b6d4 0%, #3b82f6 100%);
		background-clip: text;
		-webkit-background-clip: text;
		color: #67e8f9;
	}

	.swarm-header.swarm-build {
		color: #f97316;
	}

	.swarm-name {
		letter-spacing: 0.05em;
	}

	.swarm-agents {
		padding: 0.25rem 0;
	}

	.swarm-agent-row {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.25rem 0.75rem;
		font-family: 'JetBrains Mono', monospace;
		font-size: 11px;
	}

	.swarm-agent-status {
		width: 1rem;
		text-align: center;
		font-size: 12px;
	}

	.swarm-agent-name {
		color: #a1a1aa;
		min-width: 5rem;
		font-weight: 500;
	}

	.swarm-agent-action {
		flex: 1;
		min-width: 0;
		color: #52525b;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.swarm-prompt {
		padding: 0.375rem 0.75rem;
		border-top: 1px solid #27272a;
		font-size: 0.65rem;
		color: #52525b;
		font-family: 'JetBrains Mono', monospace;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.agent-spinner {
		display: inline-block;
		width: 10px;
		height: 10px;
		border: 1.5px solid transparent;
		border-top-color: #3b82f6;
		border-right-color: #3b82f6;
		border-radius: 50%;
		animation: agent-spin 0.8s linear infinite;
	}

	@keyframes agent-spin {
		to { transform: rotate(360deg); }
	}

	.pincer-anim {
		display: inline-block;
		font-size: 10px;
		animation: pincer 1.6s ease-in-out infinite;
	}

	.pincer-anim::after {
		content: '(\\/)';
	}

	@keyframes pincer {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.4; }
	}

	.wave-anim {
		font-size: 9px;
		letter-spacing: -1px;
		animation: wave-shift 1.2s steps(6) infinite;
		display: inline-block;
	}

	@keyframes wave-shift {
		0% { filter: hue-rotate(0deg); opacity: 0.7; }
		50% { opacity: 1; }
		100% { filter: hue-rotate(0deg); opacity: 0.7; }
	}
</style>
