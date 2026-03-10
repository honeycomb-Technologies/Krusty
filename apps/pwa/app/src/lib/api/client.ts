const API_BASE = (import.meta.env.VITE_API_BASE || '/api').replace(/\/+$/, '');

const STREAM_ACTIVITY_TIMEOUT = 30_000; // 30 seconds
const STREAM_CHECK_INTERVAL = 5_000; // Check every 5 seconds
const REMOTE_TOKEN_STORAGE_KEY = 'krusty:remote-token';
const REMOTE_TOKEN_HASH_KEY = 'krusty-remote-token';

function isLocalHost(hostname: string): boolean {
	const normalized = hostname.trim().toLowerCase();
	return normalized === 'localhost' || normalized === '127.0.0.1' || normalized === '::1';
}

function isRemoteOrigin(): boolean {
	if (typeof window === 'undefined') {
		return false;
	}
	return !isLocalHost(window.location.hostname);
}

function getRemoteAccessToken(): string | null {
	if (typeof window === 'undefined') {
		return null;
	}

	try {
		const hash = window.location.hash.startsWith('#')
			? window.location.hash.slice(1)
			: window.location.hash;
		const hashParams = new URLSearchParams(hash);
		const tokenFromHash = hashParams.get(REMOTE_TOKEN_HASH_KEY);
		if (tokenFromHash) {
			localStorage.setItem(REMOTE_TOKEN_STORAGE_KEY, tokenFromHash);
			hashParams.delete(REMOTE_TOKEN_HASH_KEY);
			const nextHash = hashParams.toString();
			const nextUrl = `${window.location.pathname}${window.location.search}${nextHash ? `#${nextHash}` : ''}`;
			window.history.replaceState({}, '', nextUrl);
			return tokenFromHash;
		}

		return localStorage.getItem(REMOTE_TOKEN_STORAGE_KEY);
	} catch {
		return null;
	}
}

// ============================================================================
// Type Definitions
// ============================================================================

/** Session info returned from API */
export interface SessionResponse {
	id: string;
	title: string;
	token_count?: number | null;
	working_dir: string | null;
	parent_session_id: string | null;
	mode: 'build' | 'plan';
	updated_at: string;
	model?: string | null;
	target_branch?: string | null;
}

/** Message content block */
export interface MessageContent {
	type: 'text' | 'tool_use' | 'tool_result' | 'thinking';
	text?: string;
	id?: string;
	name?: string;
	input?: Record<string, unknown>;
	tool_use_id?: string;
	content?: string;
	thinking?: string;
}

/** Message in a session */
export interface MessageResponse {
	role: 'user' | 'assistant';
	content: MessageContent[];
}

/** Session with messages */
export interface SessionWithMessagesResponse {
	session: SessionResponse;
	messages: MessageResponse[];
}

/** Model info */
export interface ModelInfo {
	id: string;
	display_name: string;
	provider: string;
	context_window: number;
	max_output: number;
	supports_thinking: boolean;
	supports_tools: boolean;
}

/** Models response */
export interface ModelsResponse {
	models: ModelInfo[];
	default_model: string;
}

/** File tree entry */
export interface TreeEntry {
	name: string;
	path: string;
	is_dir: boolean;
	children?: TreeEntry[];
}

/** Git status summary */
export interface GitStatusResponse {
	in_repo: boolean;
	repo_root: string | null;
	branch: string | null;
	head: string | null;
	upstream: string | null;
	branch_files: number;
	branch_additions: number;
	branch_deletions: number;
	pr_number: number | null;
	ahead: number;
	behind: number;
	staged: number;
	modified: number;
	untracked: number;
	conflicted: number;
	total_changes: number;
}

/** Git branch item */
export interface GitBranch {
	name: string;
	is_current: boolean;
	upstream: string | null;
	is_remote: boolean;
}

/** Git branch list response */
export interface GitBranchesResponse {
	repo_root: string;
	branches: GitBranch[];
}

/** Git worktree item */
export interface GitWorktree {
	path: string;
	branch: string | null;
	head: string | null;
	is_current: boolean;
}

/** Git worktree list response */
export interface GitWorktreesResponse {
	repo_root: string;
	worktrees: GitWorktree[];
}

/** Provider credential status */
export interface ProviderStatus {
	id: string;
	name: string;
	configured: boolean;
	has_oauth: boolean;
	supports_oauth: boolean;
}

/** OAuth start response */
export interface OAuthDeviceCodeInfo {
	user_code: string;
	verification_uri: string;
	verification_uri_complete?: string | null;
	expires_in: number;
}

/** OAuth start response */
export interface OAuthStartResponse {
	auth_url: string;
	provider: string;
	flow_type: 'browser_callback' | 'device' | 'paste_code';
	paste_code: boolean;
	device_code?: OAuthDeviceCodeInfo | null;
}

/** OAuth status response */
export interface OAuthStatusResponse {
	has_token: boolean;
	flow_active: boolean;
}

/** Push diagnostics summary */
export interface PushStatusResponse {
	push_configured: boolean;
	subscription_count: number;
	last_attempt_at: string | null;
	last_success_at: string | null;
	last_failure_at: string | null;
	last_failure_reason: string | null;
	recent_failures_24h: number;
}

/** Push test-send response */
export interface PushTestResponse {
	accepted: boolean;
	attempted: number;
	sent: number;
	stale_removed: number;
	failed: number;
}

/** Preview/port-forwarding settings */
export interface PreviewSettings {
	enabled: boolean;
	auto_refresh_secs: number;
	show_only_http_like: boolean;
	probe_timeout_ms: number;
	allow_force_open_non_http: boolean;
	pinned_ports: number[];
	hidden_ports: number[];
	blocked_ports: number[];
}

/** Preview settings patch */
export interface PreviewSettingsPatch {
	enabled?: boolean;
	auto_refresh_secs?: number;
	show_only_http_like?: boolean;
	probe_timeout_ms?: number;
	allow_force_open_non_http?: boolean;
	pinned_ports?: number[];
	hidden_ports?: number[];
	blocked_ports?: number[];
}

/** Discovered or pinned port entry */
export interface PortEntry {
	port: number;
	name: string;
	description: string | null;
	command: string | null;
	pid: number | null;
	source: 'discovered' | 'pinned' | 'discovered+pinned' | string;
	active: boolean;
	pinned: boolean;
	is_http_like: boolean;
	is_previewable_http: boolean;
	probe_status: 'ok' | 'timeout' | 'conn_refused' | 'non_http' | 'error' | string;
	last_probe_ms: number | null;
	preview_path: string;
}

/** Port list response */
export interface PortListResponse {
	ports: PortEntry[];
	settings: PreviewSettings;
	discovery_error?: string | null;
}

export interface RecoveryToolCall {
	id: string;
	name: string;
}

export interface SessionRecoveryState {
	schema_version: number;
	status: string;
	stop_reason: string | null;
	last_error: string | null;
	partial_assistant: {
		text: string;
		thinking?: string;
		tool_calls: RecoveryToolCall[];
	};
	decision: Record<string, unknown>;
}

export interface PartialAssistantState {
	text: string;
	thinking?: string;
	tool_calls: RecoveryToolCall[];
}

export interface SessionStateResponse {
	id: string;
	agent_state: string;
	started_at: string | null;
	last_event_at: string | null;
	mode: 'build' | 'plan';
	recovery?: SessionRecoveryState | null;
	live_partial_assistant?: PartialAssistantState | null;
	last_event_sequence?: number | null;
}

export type PresenceCapability = 'observer' | 'controller';

export interface SessionPresenceClientResponse {
	client_id: string;
	surface: string;
	capability: PresenceCapability;
	user_id?: string | null;
	last_seen_at: string;
	last_event_sequence?: number | null;
	stale: boolean;
}

export interface SessionPresenceResponse {
	session_id: string;
	active_viewers: number;
	active_controllers: number;
	stale_clients: number;
	clients: SessionPresenceClientResponse[];
}

export interface TailscaleAccessResponse {
	status: string;
	url?: string | null;
	detail?: string | null;
}

export interface ServerAccessResponse {
	local_url: string;
	remote_access_enabled: boolean;
	remote_access_token: string;
	remote_launch_url?: string | null;
	tailscale: TailscaleAccessResponse;
}

export interface ActiveSessionStatusResponse {
	id: string;
	title: string;
	agent_state: string;
	started_at?: string | null;
	last_event_at?: string | null;
	working_dir?: string | null;
	active_viewers: number;
	active_controllers: number;
	stale_clients: number;
}

export interface ServerStatusResponse {
	active_agent_streams: number;
	active_sessions: ActiveSessionStatusResponse[];
	tailscale: TailscaleAccessResponse;
}

/** SSE stream event types */
export type StreamEvent =
	| { type: 'text_delta'; delta: string }
	| { type: 'text_delta_with_citations'; delta: string; citations: unknown[] }
	| { type: 'thinking_delta'; thinking: string }
	| { type: 'thinking_complete'; thinking: string; signature: string }
	| { type: 'tool_call_start'; id: string; name: string }
	| { type: 'tool_call_complete'; id: string; name: string; arguments: Record<string, unknown> }
	| { type: 'tool_executing'; id: string; name: string }
	| { type: 'tool_output_delta'; id: string; delta: string }
	| { type: 'tool_result'; id: string; output: string; is_error: boolean }
	| { type: 'server_tool_start'; id: string; name: string }
	| { type: 'server_tool_complete'; id: string; name: string }
	| { type: 'web_search_results'; tool_use_id: string; results: unknown[] }
	| { type: 'web_fetch_result'; tool_use_id: string; content: unknown }
	| { type: 'server_tool_error'; tool_use_id: string; error_code: string }
	| { type: 'plan_update'; items: PlanItem[] }
	| { type: 'mode_change'; mode: string; reason?: string }
	| { type: 'plan_complete'; tool_call_id: string; title: string; task_count: number }
	| { type: 'usage'; prompt_tokens: number; completion_tokens: number }
	| {
			type: 'context_compacted';
			reason: string;
			estimated_tokens_before: number;
			estimated_tokens_after: number;
			replaced_messages: number;
	  }
	| { type: 'lagged'; skipped: number }
	| { type: 'title_update'; title: string }
	| { type: 'tool_approval_required'; id: string; name: string; arguments: Record<string, unknown> }
	| { type: 'tool_approved'; id: string }
	| { type: 'tool_denied'; id: string }
	| { type: 'turn_complete'; turn: number; has_more: boolean }
	| { type: 'finish'; session_id: string; stop_reason: string }
	| { type: 'error'; error: string };

// ============================================================================
// API Client
// ============================================================================

// Store for current user ID (set from auth session)
let currentUserId: string | null = null;

export function setCurrentUserId(userId: string | null) {
	currentUserId = userId;
}

export function getCurrentUserId(): string | null {
	return currentUserId;
}

export function getApiUrl(path: string): string {
	const normalized = path.startsWith('/') ? path : `/${path}`;
	return `${API_BASE}${normalized}`;
}

export async function bootstrapRemoteAccess(): Promise<void> {
	if (typeof window === 'undefined' || isRemoteOrigin()) {
		return;
	}

	try {
		const access = await apiClient.getServerAccess();
		localStorage.setItem(REMOTE_TOKEN_STORAGE_KEY, access.remote_access_token);
	} catch {
		// Best-effort only; local operation does not depend on remote bootstrap.
	}
}

class ApiError extends Error {
	constructor(
		public status: number,
		message: string
	) {
		super(message);
		this.name = 'ApiError';
	}
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
	// Build headers with optional X-User-Id for multi-tenant auth
	const headers: Record<string, string> = {
		'Content-Type': 'application/json',
		...(options.headers as Record<string, string>)
	};

	if (currentUserId) {
		headers['X-User-Id'] = currentUserId;
	}

	const remoteToken = getRemoteAccessToken();
	if (isRemoteOrigin() && remoteToken) {
		headers.Authorization = `Bearer ${remoteToken}`;
	}

	const response = await fetch(getApiUrl(path), {
		...options,
		headers
	});

	if (!response.ok) {
		const error = await response.json().catch(() => ({ error: 'Unknown error' }));
		throw new ApiError(response.status, error.error || error.message || 'Request failed');
	}

	// Handle empty responses (204 No Content, etc.)
	const contentLength = response.headers.get('content-length');
	if (response.status === 204 || contentLength === '0') {
		return undefined as T;
	}

	return response.json();
}

export const apiClient = {
	// Sessions
	getSessions: () => request<SessionResponse[]>('/sessions'),

	getSession: (id: string) => request<SessionWithMessagesResponse>(`/sessions/${id}`),

	getDirectories: () => request<string[]>('/sessions/directories'),

	browseDirectories: (path?: string) =>
		request<{
			current: string;
			parent: string | null;
			directories: { name: string; path: string }[];
		}>(`/files/browse${path ? `?path=${encodeURIComponent(path)}` : ''}`),

	createSession: (title?: string, workingDir?: string, targetBranch?: string) =>
		request<SessionResponse>('/sessions', {
			method: 'POST',
			body: JSON.stringify({ title, working_dir: workingDir, target_branch: targetBranch })
		}),

	deleteSession: (id: string) =>
		request<void>(`/sessions/${id}`, { method: 'DELETE' }),

	updateSession: (id: string, data: { title?: string; working_dir?: string; mode?: 'build' | 'plan'; model?: string; target_branch?: string }) =>
		request<SessionResponse>(`/sessions/${id}`, {
			method: 'PATCH',
			body: JSON.stringify(data)
		}),

	pinchSession: (id: string, preservationHints?: string, direction?: string) =>
		request<{
			session: SessionResponse;
			summary: string;
			key_decisions: string[];
			pending_tasks: string[];
		}>(`/sessions/${id}/pinch`, {
			method: 'POST',
			body: JSON.stringify({
				preservation_hints: preservationHints,
				direction
			})
		}),

	getSessionState: (id: string) =>
		request<SessionStateResponse>(`/sessions/${id}/state`),

	getSessionPresence: (id: string) =>
		request<SessionPresenceResponse>(`/sessions/${id}/presence`),

	heartbeatSessionPresence: (
		id: string,
		data: {
			client_id: string;
			surface: string;
			capability: PresenceCapability;
			last_event_sequence?: number | null;
		}
	) =>
		request<SessionPresenceResponse>(`/sessions/${id}/presence`, {
			method: 'PUT',
			body: JSON.stringify(data)
		}),

	removeSessionPresence: (id: string, clientId: string) =>
		request<SessionPresenceResponse>(`/sessions/${id}/presence/${encodeURIComponent(clientId)}`, {
			method: 'DELETE'
		}),

	getServerAccess: () => request<ServerAccessResponse>('/server/access'),

	updateServerAccess: (data: { enabled?: boolean; rotate_token?: boolean }) =>
		request<ServerAccessResponse>('/server/access', {
			method: 'PATCH',
			body: JSON.stringify(data)
		}),

	getServerStatus: () => request<ServerStatusResponse>('/server/status'),

	// Models
	getModels: () => request<ModelsResponse>('/models'),

	// Files
	getFile: (path: string) =>
		request<{ path: string; content: string; size: number }>(`/files?path=${encodeURIComponent(path)}`),

	writeFile: (path: string, content: string) =>
		request<{ path: string; bytes_written: number }>(`/files?path=${encodeURIComponent(path)}`, {
			method: 'PUT',
			body: JSON.stringify({ content })
		}),

	getFileTree: (root?: string, depth = 3) =>
		request<{ root: string; entries: TreeEntry[] }>(
			`/files/tree?${root ? `root=${encodeURIComponent(root)}&` : ''}depth=${depth}`
		),

	// Git
	getGitStatus: (path?: string) =>
		request<GitStatusResponse>(`/git/status${path ? `?path=${encodeURIComponent(path)}` : ''}`),

	getGitBranches: (path?: string) =>
		request<GitBranchesResponse>(`/git/branches${path ? `?path=${encodeURIComponent(path)}` : ''}`),

	getGitWorktrees: (path?: string) =>
		request<GitWorktreesResponse>(`/git/worktrees${path ? `?path=${encodeURIComponent(path)}` : ''}`),

	checkoutGitBranch: (branch: string, path?: string, create = false, startPoint?: string) =>
		request<GitStatusResponse>('/git/checkout', {
			method: 'POST',
			body: JSON.stringify({
				path,
				branch,
				create,
				start_point: startPoint
			})
		}),

	// Tools
	executeTool: (toolName: string, params: Record<string, unknown>) =>
		request<{ output: string; is_error: boolean }>('/tools/execute', {
			method: 'POST',
			body: JSON.stringify({ tool_name: toolName, params })
		}),

	// Submit a tool result (for interactive tools like AskUserQuestion)
	// Returns void - use streamToolResult for streaming continuation
	submitToolResult: (sessionId: string, toolCallId: string, result: string) =>
		request<void>('/chat/tool-result', {
			method: 'POST',
			body: JSON.stringify({ session_id: sessionId, tool_call_id: toolCallId, result })
		}),

	submitToolApproval: (sessionId: string, toolCallId: string, approved: boolean) =>
		request<{ status: string }>('/chat/tool-approval', {
			method: 'POST',
			body: JSON.stringify({ session_id: sessionId, tool_call_id: toolCallId, approved })
		}),

	// Credentials
	getCredentials: () => request<ProviderStatus[]>('/credentials'),

	setCredential: (providerId: string, apiKey: string) =>
		request<ProviderStatus>(`/credentials/${providerId}`, {
			method: 'POST',
			body: JSON.stringify({ api_key: apiKey })
		}),

	deleteCredential: (providerId: string) =>
		request<void>(`/credentials/${providerId}`, { method: 'DELETE' }),

	// OAuth
	startOAuth: (provider: string) =>
		request<OAuthStartResponse>('/auth/oauth/start', {
			method: 'POST',
			body: JSON.stringify({ provider })
		}),

	getOAuthStatus: (provider: string) =>
		request<OAuthStatusResponse>(`/auth/oauth/status/${provider}`),

	exchangeOAuthCode: (provider: string, code: string) =>
		request<{ success: boolean }>('/auth/oauth/exchange', {
			method: 'POST',
			body: JSON.stringify({ provider, code })
		}),

	revokeOAuth: (provider: string) =>
		request<OAuthStatusResponse>(`/auth/oauth/revoke/${provider}`, {
			method: 'DELETE'
		}),

	// Push notifications
	getVapidPublicKey: () =>
		request<{ public_key: string }>('/push/vapid-public-key'),

	pushSubscribe: (subscription: { endpoint: string; p256dh: string; auth: string }) =>
		request<{ id: string }>('/push/subscribe', {
			method: 'POST',
			body: JSON.stringify(subscription)
		}),

	pushUnsubscribe: (data: { endpoint: string }) =>
		request<{ removed: boolean }>('/push/subscribe', {
			method: 'DELETE',
			body: JSON.stringify(data)
		}),

	getPushStatus: () => request<PushStatusResponse>('/push/status'),

	sendPushTest: (payload: { session_id?: string; title?: string; body?: string } = {}) =>
		request<PushTestResponse>('/push/test', {
			method: 'POST',
			body: JSON.stringify(payload)
		}),

	// Preview / Port Forwarding
	getPorts: () => request<PortListResponse>('/ports'),

	getPreviewSettings: () => request<PreviewSettings>('/settings/preview'),

	updatePreviewSettings: (patch: PreviewSettingsPatch) =>
		request<PreviewSettings>('/settings/preview', {
			method: 'PATCH',
			body: JSON.stringify(patch)
		}),

	addPinnedPort: (port: number) =>
		request<PreviewSettings>('/settings/preview/pins', {
			method: 'POST',
			body: JSON.stringify({ port })
		}),

	removePinnedPort: (port: number) =>
		request<PreviewSettings>(`/settings/preview/pins/${port}`, {
			method: 'DELETE'
		}),

	addHiddenPort: (port: number) =>
		request<PreviewSettings>('/settings/preview/hidden', {
			method: 'POST',
			body: JSON.stringify({ port })
		}),

	removeHiddenPort: (port: number) =>
		request<PreviewSettings>(`/settings/preview/hidden/${port}`, {
			method: 'DELETE'
		}),
};

// Chat streaming
export interface ImageContent {
	type: 'image';
	source: {
		type: 'base64';
		media_type: string;
		data: string;
	};
}

export interface TextContent {
	type: 'text';
	text: string;
}

export type ContentBlock = TextContent | ImageContent;

interface ChatRequest {
	session_id?: string;
	message: string;
	content?: ContentBlock[]; // For multi-modal (text + images)
	model?: string;
	thinking_enabled?: boolean | string; // boolean for backward compat, string for levels (low/medium/high)
	permission_mode?: 'supervised' | 'autonomous';
	mode?: 'build' | 'plan';
}

export interface PlanItem {
	content: string;
	completed?: boolean;
}

export interface StreamCallbacks {
	onTextDelta: (delta: string) => void;
	onThinkingDelta: (thinking: string) => void;
	onToolCallStart: (id: string, name: string) => void;
	onToolCallComplete: (id: string, name: string, args: Record<string, unknown>) => void;
	onToolResult: (id: string, output: string, isError: boolean) => void;
	onToolOutputDelta: (id: string, delta: string) => void;
	onToolApprovalRequired?: (id: string, name: string, args: Record<string, unknown>) => void;
	onToolApproved?: (id: string) => void;
	onToolDenied?: (id: string) => void;
	onTurnComplete?: (turn: number, hasMore: boolean) => void;
	onPlanUpdate: (items: PlanItem[]) => void;
	onModeChange: (mode: string, reason?: string) => void;
	onPlanComplete: (toolCallId: string, title: string, taskCount: number) => void;
	onUsage: (promptTokens: number, completionTokens: number) => void;
	onTitleUpdate: (title: string) => void;
	onFinish: (sessionId: string) => void;
	onError: (error: string) => void;
}

interface ToolResultRequest {
	session_id: string;
	tool_call_id: string;
	result: string;
}

async function streamSSE(
	url: string,
	body: object,
	callbacks: StreamCallbacks,
	signal?: AbortSignal
): Promise<void> {
	const headers: Record<string, string> = { 'Content-Type': 'application/json' };
	if (currentUserId) {
		headers['X-User-Id'] = currentUserId;
	}
	const remoteToken = getRemoteAccessToken();
	if (isRemoteOrigin() && remoteToken) {
		headers.Authorization = `Bearer ${remoteToken}`;
	}

	const response = await fetch(getApiUrl(url), {
		method: 'POST',
		headers,
		body: JSON.stringify(body),
		signal
	});

	if (!response.ok) {
		const error = await response.json().catch(() => ({ error: 'Unknown error' }));
		callbacks.onError(error.error || 'Request failed');
		return;
	}

	const reader = response.body?.getReader();
	if (!reader) {
		callbacks.onError('No response body');
		return;
	}

	const decoder = new TextDecoder();
	let buffer = '';
	let lastActivity = Date.now();

	const timeoutInterval = setInterval(() => {
		if (Date.now() - lastActivity > STREAM_ACTIVITY_TIMEOUT) {
			clearInterval(timeoutInterval);
			reader.cancel().catch(() => {});
			callbacks.onError('Stream timeout: no data received for 30 seconds');
		}
	}, STREAM_CHECK_INTERVAL);

	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;

			lastActivity = Date.now();
			buffer += decoder.decode(value, { stream: true });
			const lines = buffer.split('\n');
			buffer = lines.pop() || '';

			for (const line of lines) {
				if (line.startsWith('data: ')) {
					const data = line.slice(6);
					if (data === '[DONE]') continue;

					try {
						const event = JSON.parse(data);
						handleEvent(event, callbacks);
					} catch (e) {
						console.warn('[SSE] Parse error:', data, e);
					}
				}
			}
		}
	} catch (err) {
		if (err instanceof Error && err.name === 'AbortError') {
			// User cancelled
		} else {
			callbacks.onError(err instanceof Error ? err.message : 'Stream error');
		}
	} finally {
		clearInterval(timeoutInterval);
		reader.cancel().catch(() => {});
	}
}

export async function streamToolResult(
	req: ToolResultRequest,
	callbacks: StreamCallbacks,
	signal?: AbortSignal
) {
	return streamSSE('/chat/tool-result', req, callbacks, signal);
}

export async function streamChat(
	req: ChatRequest,
	callbacks: StreamCallbacks,
	signal?: AbortSignal
) {
	return streamSSE('/chat', req, callbacks, signal);
}

function handleEvent(event: StreamEvent, callbacks: StreamCallbacks) {
	switch (event.type) {
		case 'text_delta':
			callbacks.onTextDelta(event.delta);
			break;
		case 'text_delta_with_citations':
			callbacks.onTextDelta(event.delta);
			break;
		case 'thinking_delta':
			callbacks.onThinkingDelta(event.thinking);
			break;
		case 'thinking_complete':
			break;
		case 'tool_call_start':
			callbacks.onToolCallStart(event.id, event.name);
			break;
		case 'tool_call_complete':
			callbacks.onToolCallComplete(event.id, event.name, event.arguments);
			break;
		case 'tool_executing':
			// Heartbeat - no-op (activity timeout updated by read loop)
			break;
		case 'tool_output_delta':
			callbacks.onToolOutputDelta(event.id, event.delta);
			break;
		case 'tool_result':
			callbacks.onToolResult(event.id, event.output, event.is_error);
			break;
		case 'server_tool_start':
			callbacks.onToolCallStart(event.id, event.name);
			break;
		case 'server_tool_complete':
			callbacks.onToolResult(event.id, `${event.name} completed`, false);
			break;
		case 'web_search_results':
		case 'web_fetch_result':
			break;
		case 'server_tool_error':
			callbacks.onToolResult(
				event.tool_use_id,
				`Server tool error: ${event.error_code}`,
				true
			);
			break;
		case 'plan_update':
			callbacks.onPlanUpdate(event.items);
			break;
		case 'mode_change':
			callbacks.onModeChange(event.mode, event.reason);
			break;
		case 'plan_complete':
			callbacks.onPlanComplete(event.tool_call_id, event.title, event.task_count);
			break;
		case 'usage':
			callbacks.onUsage(event.prompt_tokens, event.completion_tokens);
			break;
		case 'context_compacted':
		case 'lagged':
			break;
		case 'title_update':
			callbacks.onTitleUpdate(event.title);
			break;
		case 'tool_approval_required':
			callbacks.onToolApprovalRequired?.(event.id, event.name, event.arguments);
			break;
		case 'tool_approved':
			callbacks.onToolApproved?.(event.id);
			break;
		case 'tool_denied':
			callbacks.onToolDenied?.(event.id);
			break;
		case 'turn_complete':
			callbacks.onTurnComplete?.(event.turn, event.has_more);
			break;
		case 'finish':
			callbacks.onFinish(event.session_id);
			break;
		case 'error':
			callbacks.onError(event.error);
			break;
	}
}
