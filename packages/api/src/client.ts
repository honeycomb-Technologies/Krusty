import type {
	SessionResponse,
	SessionWithMessagesResponse,
	SessionStateResponse,
	WorkflowCommand,
	WorkflowMutation,
	WorkflowSnapshot,
	SessionPresenceResponse,
	ModelsResponse,
	GitStatusResponse,
	GitChangesResponse,
	GitFileDiffResponse,
	GitBranchesResponse,
	GitWorktreesResponse,
	ProviderStatus,
	OAuthStartResponse,
	OAuthStatusResponse,
	OAuthExchangeResponse,
	ServerAccessResponse,
	ChatRequest,
	ToolResultRequest,
	StreamCallbacks,
	StreamEvent,
	UsageMetrics,
	DelegatedProgressEvent,
	SessionType,
	TreeEntry,
	WorkspaceMode,
	PreviewSettings,
	PreviewSettingsPatch,
	PortListResponse,
	McpServerResponse,
	McpToolResponse,
	SkillInfo,
	AgentMemory,
	Report,
	ReportSummary,
	MemoryType,
	MemorySnapshotResponse,
	PromoteReportToMemoryResponse,
	MakoAttentionResponse,
	MakoDispatchOptions,
	MakoDispatchResponse,
	MakoMainResponse,
	MakoGlobalSchedule,
	MakoSchedule,
	MakoScheduleMutationResponse,
	MakoScheduleWriteRequest,
	MakoBootstrapResponse,
	MakoCrewDocumentKind,
	MakoCrewResponse,
	MakoChannelsResponse,
	MakoCurrentResponse,
	MakoHomeDocumentKind,
	MakoHomeResponse,
	MakoRecoverDaemonResponse,
	MakoRunPriority,
	MakoSessionSummary,
	PermissionMode,
	MakoSessionStatus,
	ModelKey,
	ApnsRegisterResponse,
	ApnsStatusResponse,
		SimpleOkResponse,
		SteerRequest,
		SteerResponse,
		MobileDiagnosticUploadBatch,
		MobileDiagnosticUploadResponse,
	} from "./types";

type UsageStreamEvent = Extract<StreamEvent, { type: "usage" }>;

export function normalizeUsageMetrics(event: UsageStreamEvent): UsageMetrics {
	const cacheCreationInputTokens = event.cache_creation_input_tokens ?? 0;
	const cacheReadInputTokens = event.cache_read_input_tokens ?? 0;
	const reasoningTokens = Math.max(
		0,
		Math.min(event.reasoning_tokens ?? 0, Math.max(0, event.completion_tokens)),
	);
	const inputTokens =
		event.input_tokens ??
		event.prompt_tokens + cacheCreationInputTokens + cacheReadInputTokens;
	const representedTotal = inputTokens + event.completion_tokens;

	return {
		promptTokens: event.prompt_tokens,
		inputTokens,
		completionTokens: event.completion_tokens,
		reasoningTokens,
		cacheCreationInputTokens,
		cacheReadInputTokens,
		totalTokens: Math.max(event.total_tokens ?? 0, representedTotal),
	};
}

const STREAM_ACTIVITY_TIMEOUT = 240_000;

function monotonicNow(): number {
	return globalThis.performance?.now?.() ?? Date.now();
}

function isAbortError(error: unknown): boolean {
	return error instanceof Error && error.name === "AbortError";
}

function httpStatusClass(status: number): string {
	if (status >= 200 && status < 300) return "http.2xx";
	if (status >= 300 && status < 400) return "http.3xx";
	if (status >= 400 && status < 500) return "http.4xx";
	if (status >= 500) return "http.5xx";
	return "http.unknown";
}

function requestDiagnosticName(path: string): string {
	const route = path.split("?", 1)[0] ?? path;
	if (route === "/sessions") return "api.sessions.catalog";
	if (route === "/sessions/directories") return "api.sessions.directories";
	if (route.startsWith("/sessions/")) {
		const segments = route.split("/").filter(Boolean);
		const subroute = segments[2];
		if (!subroute) return "api.sessions.detail";
		if (subroute === "state") return "api.sessions.state";
		if (subroute === "workflow") return "api.sessions.workflow";
		if (subroute === "presence") return "api.sessions.presence";
		if (subroute === "cancel" || subroute === "pinch") {
			return "api.sessions.action";
		}
		return "api.sessions";
	}
	if (route.startsWith("/models")) return "api.models";
	if (route.startsWith("/credentials")) return "api.credentials";
	if (route.startsWith("/auth")) return "api.auth";
	if (route.startsWith("/mcp")) return "api.mcp";
	if (route.startsWith("/skills")) return "api.skills";
	if (route.startsWith("/ports") || route.startsWith("/settings/preview")) {
		return "api.ports";
	}
	if (route.startsWith("/mako")) return "api.mako";
	if (route.startsWith("/git")) return "api.git";
	if (route.startsWith("/files")) return "api.files";
	if (route.startsWith("/apns") || route.startsWith("/push")) {
		return "api.notifications";
	}
	if (route.startsWith("/mobile-diagnostics")) {
		return "api.mobile_diagnostics";
	}
	if (route.startsWith("/chat")) return "api.stream";
	return "api.other";
}

function apiErrorMessage(body: string, fallback: string): string {
	if (!body) return fallback;
	try {
		const parsed = JSON.parse(body) as { error?: unknown; message?: unknown };
		if (typeof parsed.error === "string" && parsed.error.trim()) {
			return parsed.error;
		}
		if (typeof parsed.message === "string" && parsed.message.trim()) {
			return parsed.message;
		}
	} catch {
		// Plain-text provider and proxy errors are already human-readable.
	}
	return body;
}

export interface KrustyClientConfig {
	baseUrl: string;
	token?: string;
	/** Custom fetch implementation for environments without streaming support (e.g. React Native). */
	fetchImpl?: typeof fetch;
	/** Content-free request lifecycle observer for app-owned diagnostics. */
	requestObserver?: (event: KrustyRequestDiagnostic) => void;
}

export type KrustyRequestDiagnosticOutcome =
	| "start"
	| "complete"
	| "cancel"
	| "error";

export interface KrustyRequestDiagnostic {
	name: string;
	outcome: KrustyRequestDiagnosticOutcome;
	durationMs?: number;
	code?: string;
}

export class KrustyApiError extends Error {
	constructor(
		public readonly status: number,
		message: string,
		public readonly responseBody: string,
	) {
		super(`API ${status}: ${message}`);
		this.name = "KrustyApiError";
	}
}

export class KrustyClient {
	private baseUrl: string;
	private token: string | undefined;
	private fetchFn: typeof fetch;
	private requestObserver: KrustyClientConfig["requestObserver"];

	constructor(config: KrustyClientConfig) {
		this.baseUrl = config.baseUrl.replace(/\/+$/, "");
		this.token = config.token;
		this.fetchFn = config.fetchImpl ?? globalThis.fetch.bind(globalThis);
		this.requestObserver = config.requestObserver;
	}

	private headers(): Record<string, string> {
		const h: Record<string, string> = { "Content-Type": "application/json" };
		if (this.token) {
			h["Authorization"] = `Bearer ${this.token}`;
		}
		return h;
	}

	private async request<T>(
		path: string,
		options: RequestInit = {},
	): Promise<T> {
		const url = `${this.baseUrl}/api${path}`;
		const diagnosticName = requestDiagnosticName(path);
		const startedAt = monotonicNow();
		this.observeRequest(diagnosticName, "start");
		let response: Response;
		try {
			response = await this.fetchFn(url, {
				...options,
				headers: {
					...this.headers(),
					...(options.headers as Record<string, string>),
				},
			});
		} catch (error) {
			this.observeRequest(
				diagnosticName,
				isAbortError(error) ? "cancel" : "error",
				startedAt,
				isAbortError(error) ? "request.abort" : "network.error",
			);
			throw error;
		}

		if (!response.ok) {
			const text = await response.text().catch(() => "Request failed");
			const message = apiErrorMessage(text, "Request failed");
			this.observeRequest(
				diagnosticName,
				"error",
				startedAt,
				httpStatusClass(response.status),
			);
			throw new KrustyApiError(response.status, message, text);
		}

		try {
			const result = await response.json() as T;
			this.observeRequest(
				diagnosticName,
				"complete",
				startedAt,
				httpStatusClass(response.status),
			);
			return result;
		} catch (error) {
			this.observeRequest(
				diagnosticName,
				"error",
				startedAt,
				"decode.error",
			);
			throw error;
		}
	}

	private observeRequest(
		name: string,
		outcome: KrustyRequestDiagnosticOutcome,
		startedAt?: number,
		code?: string,
	): void {
		try {
			this.requestObserver?.({
				name,
				outcome,
				durationMs: startedAt === undefined
					? undefined
					: Math.max(0, monotonicNow() - startedAt),
				code,
			});
		} catch {
			// Diagnostics must never change request behavior.
		}
	}

	// Health & Auth
	async checkHealth(): Promise<boolean> {
		const diagnosticName = "api.health";
		const startedAt = monotonicNow();
		this.observeRequest(diagnosticName, "start");
		try {
			const resp = await this.fetchFn(`${this.baseUrl}/health`, {
				headers: this.headers(),
			});
			this.observeRequest(
				diagnosticName,
				resp.ok ? "complete" : "error",
				startedAt,
				httpStatusClass(resp.status),
			);
			return resp.ok;
		} catch (error) {
			this.observeRequest(
				diagnosticName,
				isAbortError(error) ? "cancel" : "error",
				startedAt,
				isAbortError(error) ? "request.abort" : "network.error",
			);
			return false;
		}
	}

	async bootstrapAuth(): Promise<boolean> {
		// The current server authenticates each API request with the bearer token
		// directly and does not expose a separate bootstrap route.
		return true;
	}

	// Notifications
	async registerApnsDevice(
		deviceToken: string,
		bundleId?: string,
		notificationLevel?: import("./types").NotificationDeliveryLevel,
		environment?: "sandbox" | "production",
	): Promise<ApnsRegisterResponse> {
		return this.request("/apns/register", {
			method: "POST",
			body: JSON.stringify({
				device_token: deviceToken,
				bundle_id: bundleId ?? undefined,
				notification_level: notificationLevel ?? undefined,
				environment: environment ?? undefined,
			}),
		});
	}

	async registerExpoPushDevice(
		expoPushToken: string,
		platform: import("./types").PushPlatform,
		notificationLevel: import("./types").NotificationDeliveryLevel,
	): Promise<import("./types").ExpoPushRegisterResponse> {
		return this.request("/push/expo/register", {
			method: "POST",
			body: JSON.stringify({
				expo_push_token: expoPushToken,
				platform,
				notification_level: notificationLevel,
			}),
		});
	}

	async unregisterExpoPushDevice(
		expoPushToken: string,
	): Promise<{ removed: boolean }> {
		return this.request("/push/expo/register", {
			method: "DELETE",
			body: JSON.stringify({
				expo_push_token: expoPushToken,
			}),
		});
	}

	async uploadMobileDiagnostics(
		batch: MobileDiagnosticUploadBatch,
	): Promise<MobileDiagnosticUploadResponse> {
		return this.request("/mobile-diagnostics/batches", {
			method: "POST",
			body: JSON.stringify(batch),
		});
	}

	async registerLiveActivity(
		request: import("./types").LiveActivityRegisterRequest,
	): Promise<ApnsRegisterResponse> {
		return this.request("/apns/live-activities/register", {
			method: "POST",
			body: JSON.stringify({
				session_id: request.sessionId,
				push_token: request.pushToken,
				content_state: request.contentState,
				started_at_ms: request.startedAtMs,
				bundle_id: request.bundleId,
				environment: request.environment,
			}),
		});
	}

	async updateLiveActivityState(
		sessionId: string,
		pushToken: string,
		contentState: Record<string, unknown>,
	): Promise<{ updated: boolean }> {
		return this.request("/apns/live-activities/state", {
			method: "POST",
			body: JSON.stringify({
				session_id: sessionId,
				push_token: pushToken,
				content_state: contentState,
			}),
		});
	}

	async unregisterLiveActivity(
		sessionId: string,
		pushToken: string,
	): Promise<{ removed: boolean }> {
		return this.request("/apns/live-activities/unregister", {
			method: "POST",
			body: JSON.stringify({
				session_id: sessionId,
				push_token: pushToken,
			}),
		});
	}

	async getApnsStatus(): Promise<ApnsStatusResponse> {
		return this.request("/apns/status");
	}

	// Sessions
	async getSessions(): Promise<SessionResponse[]> {
		return this.request("/sessions");
	}

	async getSession(id: string): Promise<SessionWithMessagesResponse> {
		return this.request(`/sessions/${id}`);
	}

	async createSession(
		title?: string,
		projectDir?: string,
		targetBranch?: string | null,
		workspaceMode?: WorkspaceMode,
		sessionType?: SessionType,
		permissionMode?: PermissionMode,
	): Promise<SessionResponse> {
		return this.request("/sessions", {
			method: "POST",
			body: JSON.stringify({
				title: title ?? undefined,
				project_dir: projectDir ?? undefined,
				target_branch: targetBranch ?? undefined,
				workspace_mode: workspaceMode ?? undefined,
				session_type: sessionType ?? undefined,
				permission_mode: permissionMode ?? undefined,
			}),
		});
	}

	async updateSession(
		id: string,
		data: Partial<{
			title: string;
			mode: string;
			model: string | null;
			model_key: ModelKey | null;
			target_branch: string | null;
			targetBranch: string | null;
			permission_mode: PermissionMode;
		}>,
	): Promise<SessionResponse> {
		return this.request(`/sessions/${id}`, {
			method: "PATCH",
			body: JSON.stringify(data),
		});
	}

	async deleteSession(id: string): Promise<void> {
		await this.request(`/sessions/${id}`, { method: "DELETE" });
	}

	async getSessionState(id: string): Promise<SessionStateResponse> {
		return this.request(`/sessions/${id}/state`);
	}

	async getWorkflow(id: string): Promise<WorkflowSnapshot | null> {
		return this.request(`/sessions/${id}/workflow`);
	}

	async executeWorkflowCommand(
		id: string,
		command: WorkflowCommand,
	): Promise<WorkflowMutation> {
		return this.request(`/sessions/${id}/workflow/commands`, {
			method: "POST",
			body: JSON.stringify(command),
		});
	}

	// Presence
	async getSessionPresence(id: string): Promise<SessionPresenceResponse> {
		return this.request(`/sessions/${id}/presence`);
	}

	async heartbeatPresence(
		sessionId: string,
		clientId: string,
		surface: string,
		capability: string,
		lastEventSequence?: number | null,
	): Promise<SessionPresenceResponse> {
		return this.request(`/sessions/${sessionId}/presence`, {
			method: "PUT",
			body: JSON.stringify({
				client_id: clientId,
				surface,
				capability,
				last_event_sequence: lastEventSequence ?? undefined,
			}),
		});
	}

	// Models
	async getModels(): Promise<ModelsResponse> {
		return this.request("/models");
	}

	async setCurrentModel(
		model: string | null,
		modelKey?: ModelKey | null,
	): Promise<SimpleOkResponse> {
		return this.request("/models/current", {
			method: "PUT",
			body: JSON.stringify({
				model,
				model_key: modelKey ?? undefined,
			}),
		});
	}

	// Git
	async getGitStatus(path?: string): Promise<GitStatusResponse> {
		const q = path ? `?path=${encodeURIComponent(path)}` : "";
		return this.request(`/git/status${q}`);
	}

	async getGitChanges(path?: string): Promise<GitChangesResponse> {
		const q = path ? `?path=${encodeURIComponent(path)}` : "";
		return this.request(`/git/changes${q}`);
	}

	async getGitFileDiff(
		file: string,
		path?: string,
	): Promise<GitFileDiffResponse> {
		const params = new URLSearchParams({ file });
		if (path) params.set("path", path);
		return this.request(`/git/diff?${params}`);
	}

	async getGitBranches(path?: string): Promise<GitBranchesResponse> {
		const q = path ? `?path=${encodeURIComponent(path)}` : "";
		return this.request(`/git/branches${q}`);
	}

	async getGitWorktrees(path?: string): Promise<GitWorktreesResponse> {
		const q = path ? `?path=${encodeURIComponent(path)}` : "";
		return this.request(`/git/worktrees${q}`);
	}

	async checkoutGitBranch(
		branch: string,
		path?: string,
		create?: boolean,
		startPoint?: string,
	): Promise<GitStatusResponse> {
		return this.request("/git/checkout", {
			method: "POST",
			body: JSON.stringify({
				branch,
				path: path ?? undefined,
				create: create ?? undefined,
				start_point: startPoint ?? undefined,
			}),
		});
	}

	// Credentials
	async getCredentials(): Promise<ProviderStatus[]> {
		return this.request("/credentials");
	}

	async setCredential(
		providerId: string,
		apiKey: string,
	): Promise<ProviderStatus> {
		return this.request(`/credentials/${providerId}`, {
			method: "POST",
			body: JSON.stringify({ api_key: apiKey }),
		});
	}

	async deleteCredential(providerId: string): Promise<void> {
		await this.request(`/credentials/${providerId}`, { method: "DELETE" });
	}

	async startOAuth(provider: string): Promise<OAuthStartResponse> {
		return this.request("/auth/oauth/start", {
			method: "POST",
			body: JSON.stringify({ provider }),
		});
	}

	async getOAuthStatus(provider: string): Promise<OAuthStatusResponse> {
		return this.request(`/auth/oauth/status/${provider}`);
	}

	async exchangeOAuthCode(
		provider: string,
		code: string,
	): Promise<OAuthExchangeResponse> {
		return this.request("/auth/oauth/exchange", {
			method: "POST",
			body: JSON.stringify({ provider, code }),
		});
	}

	async revokeOAuth(provider: string): Promise<OAuthStatusResponse> {
		return this.request(`/auth/oauth/revoke/${provider}`, { method: "DELETE" });
	}

	// Files
	async getFileTree(
		root?: string,
		depth = 3,
	): Promise<{ root: string; entries: TreeEntry[] }> {
		const params = new URLSearchParams();
		if (root) params.set("root", root);
		params.set("depth", String(depth));
		return this.request(`/files/tree?${params}`);
	}

	async getFile(
		path: string,
	): Promise<{ path: string; content: string; size: number }> {
		return this.request(`/files?path=${encodeURIComponent(path)}`);
	}

	// Server
	async getServerAccess(): Promise<ServerAccessResponse> {
		return this.request("/server/access");
	}

	// Preview / Ports
	async getPorts(): Promise<PortListResponse> {
		return this.request("/ports");
	}

	async getPreviewSettings(): Promise<PreviewSettings> {
		return this.request("/settings/preview");
	}

	async updatePreviewSettings(
		patch: PreviewSettingsPatch,
	): Promise<PreviewSettings> {
		return this.request("/settings/preview", {
			method: "PATCH",
			body: JSON.stringify(patch),
		});
	}

	async addPinnedPort(port: number): Promise<PreviewSettings> {
		return this.request("/settings/preview/pins", {
			method: "POST",
			body: JSON.stringify({ port }),
		});
	}

	async removePinnedPort(port: number): Promise<PreviewSettings> {
		return this.request(`/settings/preview/pins/${port}`, { method: "DELETE" });
	}

	async addHiddenPort(port: number): Promise<PreviewSettings> {
		return this.request("/settings/preview/hidden", {
			method: "POST",
			body: JSON.stringify({ port }),
		});
	}

	async removeHiddenPort(port: number): Promise<PreviewSettings> {
		return this.request(`/settings/preview/hidden/${port}`, {
			method: "DELETE",
		});
	}

	// MCP
	async getMcpServers(): Promise<McpServerResponse[]> {
		return this.request("/mcp");
	}

	async reloadMcpConfig(): Promise<McpServerResponse[]> {
		return this.request("/mcp/reload", { method: "POST" });
	}

	async connectMcpServer(name: string): Promise<McpServerResponse> {
		return this.request(`/mcp/${encodeURIComponent(name)}/connect`, {
			method: "POST",
		});
	}

	async disconnectMcpServer(name: string): Promise<McpServerResponse> {
		return this.request(`/mcp/${encodeURIComponent(name)}/disconnect`, {
			method: "POST",
		});
	}

	async getMcpServerTools(name: string): Promise<McpToolResponse[]> {
		return this.request(`/mcp/${encodeURIComponent(name)}/tools`);
	}

	// Skills
	async getSkills(scope: "all" | "global" = "all"): Promise<SkillInfo[]> {
		const query = scope === "global" ? "?scope=global" : "";
		return this.request(`/skills${query}`);
	}

	async updateSkillPolicy(
		name: string,
		update: { enabled?: boolean; permission?: SkillInfo["permission"] },
	): Promise<SkillInfo> {
		return this.request(`/skills/${encodeURIComponent(name)}/policy`, {
			method: "POST",
			body: JSON.stringify(update),
		});
	}

	// Tools
	async steerSession(request: SteerRequest): Promise<SteerResponse> {
		return this.request("/chat/steer", {
			method: "POST",
			body: JSON.stringify(request),
		});
	}

	async submitToolApproval(
		sessionId: string,
		toolCallId: string,
		approved: boolean,
		idempotencyKey?: string,
	): Promise<{ status: string }> {
		return this.request("/chat/tool-approval", {
			method: "POST",
			headers: idempotencyKey
				? { "Idempotency-Key": idempotencyKey }
				: undefined,
			body: JSON.stringify({
				session_id: sessionId,
				tool_call_id: toolCallId,
				approved,
			}),
		});
	}

	async cancelSession(sessionId: string): Promise<SimpleOkResponse> {
		return this.request(`/sessions/${encodeURIComponent(sessionId)}/cancel`, {
			method: "POST",
		});
	}

	async submitToolResult(
		sessionId: string,
		toolCallId: string,
		result: string,
		fastMode = false,
		permissionMode?: ToolResultRequest["permission_mode"],
	): Promise<void> {
		await this.request("/chat/tool-result", {
			method: "POST",
			body: JSON.stringify({
				session_id: sessionId,
				tool_call_id: toolCallId,
				result,
				fast_mode: fastMode,
				permission_mode: permissionMode,
			}),
		});
	}

	// Presence (aliases for backward compat)
	async heartbeatSessionPresence(
		sessionId: string,
		data: {
			client_id: string;
			surface: string;
			capability: string;
			last_event_sequence?: number | null;
		},
	): Promise<SessionPresenceResponse> {
		return this.heartbeatPresence(
			sessionId,
			data.client_id,
			data.surface,
			data.capability,
			data.last_event_sequence,
		);
	}

	async removeSessionPresence(
		sessionId: string,
		clientId: string,
	): Promise<SessionPresenceResponse> {
		return this.request(`/sessions/${sessionId}/presence/${clientId}`, {
			method: "DELETE",
		});
	}

	// Pinch
	async pinchSession(
		sessionId: string,
		preservationHints?: string,
		direction?: string,
	): Promise<{
		session: SessionResponse;
		summary: string;
		key_decisions: string[];
		pending_tasks: string[];
		estimated_tokens_before?: number;
		estimated_tokens_after?: number;
		replaced_messages?: number;
		checkpoint_id?: string;
		compaction_count?: number;
	}> {
		return this.request(`/sessions/${sessionId}/pinch`, {
			method: "POST",
			body: JSON.stringify({
				preservation_hints: preservationHints,
				direction,
			}),
		});
	}

	// Directories
	async getDirectories(): Promise<string[]> {
		return this.request("/sessions/directories");
	}

	async browseDirectories(path?: string): Promise<{
		current: string;
		parent: string | null;
		directories: Array<{ name: string; path: string }>;
	}> {
		const q = path ? `?path=${encodeURIComponent(path)}` : "";
		return this.request(`/files/browse${q}`);
	}

	// Reports
	async getReports(options?: {
		projectDir?: string;
		query?: string;
	}): Promise<{ reports: ReportSummary[] }> {
		const params: string[] = [];
		if (options?.projectDir) {
			params.push(`project_dir=${encodeURIComponent(options.projectDir)}`);
		}
		if (options?.query) {
			params.push(`query=${encodeURIComponent(options.query)}`);
		}
		const q = params.length > 0 ? `?${params.join("&")}` : "";
		return this.request(`/reports${q}`);
	}

	async getReport(id: string): Promise<Report> {
		return this.request(`/reports/${id}`);
	}

	async promoteReportToMemory(
		id: string,
		options?: { memoryType?: MemoryType },
	): Promise<PromoteReportToMemoryResponse> {
		return this.request(`/reports/${id}/promote`, {
			method: "POST",
			body: JSON.stringify({
				memory_type: options?.memoryType ?? undefined,
			}),
		});
	}

	async getMemories(
		projectDir?: string,
		memoryType?: MemoryType,
		options?: { includeContent?: boolean },
	): Promise<{ memories: AgentMemory[] }> {
		const params: string[] = [];
		if (projectDir) {
			params.push(`project_dir=${encodeURIComponent(projectDir)}`);
		}
		if (memoryType) {
			params.push(`memory_type=${encodeURIComponent(memoryType)}`);
		}
		if (options?.includeContent) {
			params.push("include_content=true");
		}
		const q = params.join("&");
		return this.request(`/memories${q ? `?${q}` : ""}`);
	}

	async getMemorySnapshot(
		projectDir?: string,
	): Promise<MemorySnapshotResponse> {
		const q = projectDir
			? `?project_dir=${encodeURIComponent(projectDir)}`
			: "";
		return this.request(`/memories/snapshot${q}`);
	}

	// Mako
	async dispatchMako(
		task: string,
		options?: MakoDispatchOptions,
	): Promise<MakoDispatchResponse> {
		return this.request("/mako/dispatch", {
			method: "POST",
			body: JSON.stringify({
				task,
				project_dir: options?.projectDir ?? undefined,
				model: options?.model ?? undefined,
				model_key: options?.modelKey ?? undefined,
				start_at: options?.startAt ?? undefined,
				priority: options?.priority ?? undefined,
				crew_slug: options?.crewSlug ?? undefined,
			}),
		});
	}

	/** Ensure/get the durable singleton Mako companion chat for this user. */
	async getMakoMain(): Promise<MakoMainResponse> {
		return this.request("/mako/main");
	}

	/** Same as getMakoMain — POST is accepted for ensure semantics. */
	async ensureMakoMain(): Promise<MakoMainResponse> {
		return this.request("/mako/main", { method: "POST" });
	}

	/**
	 * User-scoped global schedule list for the Mako Schedule secondary surface.
	 * Ordered by next fire time across all of the caller's controllers.
	 */
	async listMakoSchedules(options?: {
		limit?: number;
	}): Promise<MakoGlobalSchedule[]> {
		const params: string[] = [];
		if (options?.limit != null) {
			params.push(`limit=${encodeURIComponent(String(options.limit))}`);
		}
		const q = params.length > 0 ? `?${params.join("&")}` : "";
		return this.request(`/mako/schedules${q}`);
	}

	/** List schedules attached to a specific Mako controller session. */
	async listMakoSessionSchedules(
		sessionId: string,
		options?: { limit?: number },
	): Promise<MakoSchedule[]> {
		const params: string[] = [];
		if (options?.limit != null) {
			params.push(`limit=${encodeURIComponent(String(options.limit))}`);
		}
		const q = params.length > 0 ? `?${params.join("&")}` : "";
		return this.request(
			`/mako/sessions/${encodeURIComponent(sessionId)}/schedules${q}`,
		);
	}

	async createMakoSchedule(
		sessionId: string,
		request: MakoScheduleWriteRequest,
		options?: { idempotencyKey?: string },
	): Promise<MakoScheduleMutationResponse> {
		const headers: Record<string, string> = {};
		if (options?.idempotencyKey) {
			headers["Idempotency-Key"] = options.idempotencyKey;
		}
		return this.request(
			`/mako/sessions/${encodeURIComponent(sessionId)}/schedules`,
			{
				method: "POST",
				headers,
				body: JSON.stringify(request),
			},
		);
	}

	async pauseMakoSchedule(
		sessionId: string,
		scheduleId: string,
		revision: number,
		options?: { idempotencyKey?: string },
	): Promise<MakoScheduleMutationResponse> {
		const headers: Record<string, string> = {
			"If-Match": `"${revision}"`,
		};
		if (options?.idempotencyKey) {
			headers["Idempotency-Key"] = options.idempotencyKey;
		}
		return this.request(
			`/mako/sessions/${encodeURIComponent(sessionId)}/schedules/${encodeURIComponent(scheduleId)}/pause`,
			{ method: "POST", headers },
		);
	}

	async resumeMakoSchedule(
		sessionId: string,
		scheduleId: string,
		revision: number,
		options?: { idempotencyKey?: string },
	): Promise<MakoScheduleMutationResponse> {
		const headers: Record<string, string> = {
			"If-Match": `"${revision}"`,
		};
		if (options?.idempotencyKey) {
			headers["Idempotency-Key"] = options.idempotencyKey;
		}
		return this.request(
			`/mako/sessions/${encodeURIComponent(sessionId)}/schedules/${encodeURIComponent(scheduleId)}/resume`,
			{ method: "POST", headers },
		);
	}

	async getMakoCurrent(): Promise<MakoCurrentResponse> {
		return this.request("/mako/current");
	}

	async getMakoAttention(options?: {
		threadSessionId?: string | null;
	}): Promise<MakoAttentionResponse> {
		const params: string[] = [];
		if (options?.threadSessionId) {
			params.push(
				`thread_session_id=${encodeURIComponent(options.threadSessionId)}`,
			);
		}
		const q = params.length > 0 ? `?${params.join("&")}` : "";
		return this.request(`/mako/attention${q}`);
	}

	async setMakoAttentionRead(
		itemId: string,
		read: boolean,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/attention/${encodeURIComponent(itemId)}/read`, {
			method: "POST",
			body: JSON.stringify({ read }),
		});
	}

	async setMakoAttentionCleared(
		itemId: string,
		cleared: boolean,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/attention/${encodeURIComponent(itemId)}/clear`, {
			method: "POST",
			body: JSON.stringify({ cleared }),
		});
	}

	async getMakoHome(): Promise<MakoHomeResponse> {
		return this.request("/mako/home");
	}

	async bootstrapMakoHome(): Promise<MakoBootstrapResponse> {
		return this.request("/mako/home/bootstrap", { method: "POST" });
	}

	async updateMakoHomeDocument(
		kind: MakoHomeDocumentKind,
		content: string,
	): Promise<MakoHomeResponse> {
		return this.request(`/mako/home/${encodeURIComponent(kind)}`, {
			method: "PUT",
			body: JSON.stringify({ content }),
		});
	}

	async updateMakoCrewDocument(
		slug: string,
		kind: MakoCrewDocumentKind,
		content: string,
	): Promise<MakoHomeResponse> {
		return this.request(
			`/mako/home/crew/${encodeURIComponent(slug)}/${encodeURIComponent(kind)}`,
			{
				method: "PUT",
				body: JSON.stringify({ content }),
			},
		);
	}

	async getMakoCrew(): Promise<MakoCrewResponse> {
		return this.request("/mako/crew");
	}

	async getMakoChannels(): Promise<MakoChannelsResponse> {
		return this.request("/mako/channels");
	}

	async recoverMakoDaemon(): Promise<MakoRecoverDaemonResponse> {
		return this.request("/mako/daemon/recover", { method: "POST" });
	}

	async listMakoSessions(): Promise<MakoSessionSummary[]> {
		return this.request("/mako/sessions");
	}

	async getMakoSessionStatus(id: string): Promise<MakoSessionStatus> {
		return this.request(`/mako/sessions/${id}/status`);
	}

	async sendMakoMessage(
		id: string,
		message: string,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/message`, {
			method: "POST",
			body: JSON.stringify({ message }),
		});
	}

	async scheduleMakoSession(
		id: string,
		startAt: string,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/schedule`, {
			method: "POST",
			body: JSON.stringify({ start_at: startAt }),
		});
	}

	async setMakoSessionPriority(
		id: string,
		priority: MakoRunPriority,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/priority`, {
			method: "POST",
			body: JSON.stringify({ priority }),
		});
	}

	async setMakoSessionCrew(
		id: string,
		crewSlug?: string | null,
	): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/crew`, {
			method: "POST",
			body: JSON.stringify({ crew_slug: crewSlug ?? null }),
		});
	}

	async pauseMakoSession(id: string): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/pause`, { method: "POST" });
	}

	async resumeMakoSession(id: string): Promise<SimpleOkResponse> {
		return this.request(`/mako/sessions/${id}/resume`, { method: "POST" });
	}

	async cancelMakoSession(id: string): Promise<void> {
		await this.request(`/mako/sessions/${id}`, { method: "DELETE" });
	}

	async observeMakoSession(
		id: string,
		callbacks: StreamCallbacks,
		signal?: AbortSignal,
	): Promise<void> {
		return this.streamSSERequest(
			`/mako/sessions/${id}/events`,
			"GET",
			undefined,
			callbacks,
			signal,
		);
	}

	// ============================================================================
	// Streaming
	// ============================================================================

	async streamChat(
		request: ChatRequest,
		callbacks: StreamCallbacks,
		signal?: AbortSignal,
	): Promise<void> {
		return this.streamSSERequest("/chat", "POST", request, callbacks, signal);
	}

	async streamToolResult(
		request: ToolResultRequest,
		callbacks: StreamCallbacks,
		signal?: AbortSignal,
	): Promise<void> {
		return this.streamSSERequest(
			"/chat/tool-result",
			"POST",
			request,
			callbacks,
			signal,
		);
	}

	private async streamSSERequest(
		path: string,
		method: "GET" | "POST",
		body: object | undefined,
		callbacks: StreamCallbacks,
		signal?: AbortSignal,
	): Promise<void> {
		const url = `${this.baseUrl}/api${path}`;
		const diagnosticName = "api.stream";
		const startedAt = monotonicNow();
		this.observeRequest(diagnosticName, "start");
		let response: Response;
		try {
			response = await this.fetchFn(url, {
				method,
				headers: {
					...this.headers(),
					Accept: "text/event-stream",
				},
				body: body ? JSON.stringify(body) : undefined,
				signal,
			});
		} catch (error) {
			this.observeRequest(
				diagnosticName,
				signal?.aborted ? "cancel" : "error",
				startedAt,
				signal?.aborted ? "request.abort" : "network.error",
			);
			if (signal?.aborted) return;
			callbacks.onError(error instanceof Error ? error.message : "Stream error");
			return;
		}

		if (!response.ok) {
			this.observeRequest(
				diagnosticName,
				"error",
				startedAt,
				httpStatusClass(response.status),
			);
			const text = await response.text().catch(() => "Stream failed");
			callbacks.onError(
				`API ${response.status}: ${apiErrorMessage(text, "Stream failed")}`,
			);
			return;
		}
		this.observeRequest(
			diagnosticName,
			"complete",
			startedAt,
			httpStatusClass(response.status),
		);

		let terminalEventSeen = false;
		let errorReported = false;
		const trackedCallbacks: StreamCallbacks = {
			...callbacks,
			onFinish: (sessionId) => {
				terminalEventSeen = true;
				callbacks.onFinish(sessionId);
			},
			onError: (error) => {
				terminalEventSeen = true;
				if (errorReported) return;
				errorReported = true;
				callbacks.onError(error);
			},
		};
		const reportPrematureEnd = () => {
			if (!signal?.aborted && !terminalEventSeen) {
				trackedCallbacks.onError(
					"Stream ended before the server reported completion. Recovering the session state.",
				);
			}
		};

		const reader = response.body?.getReader?.();
		if (!reader) {
			const fallbackText = await response.text().catch(() => "");
			if (!fallbackText) {
				trackedCallbacks.onError("No response body");
				return;
			}
			this.processSSEChunk(fallbackText, trackedCallbacks, true);
			reportPrematureEnd();
			return;
		}

		const decoder = new TextDecoder();
		let buffer = "";
		let lastActivity = Date.now();

		const activityCheck = setInterval(() => {
			if (Date.now() - lastActivity > STREAM_ACTIVITY_TIMEOUT) {
				trackedCallbacks.onError(
					`Stream timeout — no activity for ${Math.round(STREAM_ACTIVITY_TIMEOUT / 1000)}s`,
				);
				reader.cancel();
				clearInterval(activityCheck);
			}
		}, 5000);

		try {
			while (true) {
				const { done, value } = await reader.read();
				if (done) break;

				lastActivity = Date.now();
				buffer = this.processSSEChunk(
					buffer + decoder.decode(value, { stream: true }),
					trackedCallbacks,
				);
			}

			buffer += decoder.decode();
			this.processSSEChunk(buffer, trackedCallbacks, true);
			reportPrematureEnd();
		} catch (err) {
			if (signal?.aborted) return;
			if (!terminalEventSeen) {
				trackedCallbacks.onError(
					err instanceof Error ? err.message : "Stream error",
				);
			}
		} finally {
			clearInterval(activityCheck);
		}
	}

	private processSSEChunk(
		chunk: string,
		callbacks: StreamCallbacks,
		flush = false,
	): string {
		let remainder = chunk;
		const eventBoundary = /\r?\n\r?\n/;

		while (true) {
			const match = eventBoundary.exec(remainder);
			if (!match) break;
			this.processSSEEvent(remainder.slice(0, match.index), callbacks);
			remainder = remainder.slice(match.index + match[0].length);
		}

		if (flush && remainder) {
			this.processSSEEvent(remainder, callbacks);
			return "";
		}

		return remainder;
	}

	private processSSEEvent(block: string, callbacks: StreamCallbacks): void {
		const data: string[] = [];
		for (const rawLine of block.split(/\r?\n/)) {
			if (!rawLine || rawLine.startsWith(":")) continue;
			const colon = rawLine.indexOf(":");
			const field = colon === -1 ? rawLine : rawLine.slice(0, colon);
			if (field !== "data") continue;
			let value = colon === -1 ? "" : rawLine.slice(colon + 1);
			if (value.startsWith(" ")) value = value.slice(1);
			data.push(value);
		}

		if (data.length === 0) return;
		const payload = data.join("\n");
		if (!payload) return;

		try {
			const event = JSON.parse(payload) as StreamEvent;
			this.handleEvent(event, callbacks);
		} catch {
			// A malformed event is isolated to its own SSE record. The next
			// record remains parseable and can still complete the stream.
		}
	}

	private handleEvent(event: StreamEvent, callbacks: StreamCallbacks): void {
		switch (event.type) {
			case "text_delta":
			case "text_delta_with_citations":
				callbacks.onTextDelta(event.delta);
				break;
			case "thinking_delta":
				callbacks.onThinkingDelta(event.thinking);
				break;
			case "tool_call_start":
				callbacks.onToolCallStart(event.id, event.name);
				break;
			case "tool_call_complete":
				callbacks.onToolCallComplete(event.id, event.name, event.arguments);
				break;
			case "tool_result":
				callbacks.onToolResult(event.id, event.output, event.is_error);
				break;
			case "tool_output_delta":
				callbacks.onToolOutputDelta(event.id, event.delta);
				break;
			case "delegated_progress":
				callbacks.onDelegatedProgress?.(event as DelegatedProgressEvent);
				break;
			case "tool_approval_required":
				callbacks.onToolApprovalRequired?.(
					event.id,
					event.name,
					event.arguments,
				);
				break;
			case "tool_approved":
				callbacks.onToolApproved?.(event.id);
				break;
			case "tool_denied":
				callbacks.onToolDenied?.(event.id);
				break;
			case "steering_injected":
				callbacks.onSteeringInjected?.(event.pending_id, event.message);
				break;
			case "turn_complete":
				callbacks.onTurnComplete?.(event.turn, event.has_more);
				break;
			case "plan_update":
				callbacks.onPlanUpdate(event.items);
				break;
			case "workflow_updated":
				callbacks.onWorkflowUpdated?.(
					event.goal_id,
					event.aggregate_revision,
					event.operation_id,
				);
				break;
			case "mode_change":
				callbacks.onModeChange(event.mode, event.reason);
				break;
			case "plan_complete":
				callbacks.onPlanComplete(
					event.tool_call_id,
					event.title,
					event.task_count,
				);
				break;
			case "usage":
				callbacks.onUsage(
					event.prompt_tokens,
					event.completion_tokens,
					normalizeUsageMetrics(event),
				);
				break;
			case "lagged":
				callbacks.onLagged?.(event.skipped);
				break;
			case "context_compaction_started":
				callbacks.onContextCompactionStarted?.(event);
				break;
			case "session_pinched":
			case "context_compacted":
				callbacks.onSessionPinched?.(event);
				break;
			case "title_update":
				callbacks.onTitleUpdate(event.title);
				break;
			case "finish":
				callbacks.onFinish(event.session_id);
				break;
			case "error":
				callbacks.onError(event.error);
				break;
			// Mako autonomous agent events
			case "user_message":
				callbacks.onUserMessage?.(event.title, event.message, event.level);
				break;
			case "agent_sleeping":
				callbacks.onAgentSleeping?.(event.duration_secs, event.reason);
				break;
			case "tick_injected":
				callbacks.onTickInjected?.(event.tick_number);
				break;
			case "classifier_decision":
				callbacks.onClassifierDecision?.(
					event.tool_name,
					event.decision,
					event.reason,
					event.stage,
				);
				break;
			case "teammate_spawned":
				callbacks.onTeammateSpawned?.(event.name, event.role);
				break;
			case "teammate_task_completed":
				callbacks.onTeammateTaskCompleted?.(
					event.name,
					event.task_id,
					event.result,
				);
				break;
			case "teammate_task_failed":
				callbacks.onTeammateTaskFailed?.(
					event.name,
					event.task_id,
					event.error,
				);
				break;
			case "teammate_cancelled":
				callbacks.onTeammateCancelled?.(event.name);
				break;
		}
	}
}
