import type {
  SessionResponse,
  SessionWithMessagesResponse,
  SessionStateResponse,
  SessionPresenceResponse,
  ModelsResponse,
  GitStatusResponse,
  GitBranchesResponse,
  GitWorktreesResponse,
  ProviderStatus,
  OAuthStartResponse,
  OAuthStatusResponse,
  OAuthExchangeResponse,
  ServerAccessResponse,
  ChatRequest,
  StreamCallbacks,
  StreamEvent,
  DelegatedProgressEvent,
  PlanItem,
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
  MakoDispatchResponse,
  MakoCurrentResponse,
  MakoRunPriority,
  MakoSessionSummary,
  MakoSessionStatus,
  ApnsRegisterResponse,
  ApnsStatusResponse,
  SimpleOkResponse,
} from './types';

const STREAM_ACTIVITY_TIMEOUT = 30_000;

export interface KrustyClientConfig {
  baseUrl: string;
  token?: string;
  /** Custom fetch implementation for environments without streaming support (e.g. React Native). */
  fetchImpl?: typeof fetch;
}

export class KrustyClient {
  private baseUrl: string;
  private token: string | undefined;
  private fetchFn: typeof fetch;

  constructor(config: KrustyClientConfig) {
    this.baseUrl = config.baseUrl.replace(/\/+$/, '');
    this.token = config.token;
    this.fetchFn = config.fetchImpl ?? globalThis.fetch.bind(globalThis);
  }

  private headers(): Record<string, string> {
    const h: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.token) {
      h['Authorization'] = `Bearer ${this.token}`;
    }
    return h;
  }

  private async request<T>(path: string, options: RequestInit = {}): Promise<T> {
    const url = `${this.baseUrl}/api${path}`;
    const response = await this.fetchFn(url, {
      ...options,
      headers: { ...this.headers(), ...(options.headers as Record<string, string>) },
    });

    if (!response.ok) {
      const text = await response.text().catch(() => 'Request failed');
      let message = text;
      try {
        const parsed = JSON.parse(text);
        message = parsed.error || parsed.message || text;
      } catch { /* use raw text */ }
      throw new Error(`API ${response.status}: ${message}`);
    }

    return response.json() as Promise<T>;
  }

  // Health & Auth
  async checkHealth(): Promise<boolean> {
    try {
      const resp = await fetch(`${this.baseUrl}/health`, { headers: this.headers() });
      return resp.ok;
    } catch {
      return false;
    }
  }

  async bootstrapAuth(): Promise<boolean> {
    // The current server authenticates each API request with the bearer token
    // directly and does not expose a separate bootstrap route.
    return true;
  }

  // Notifications
  async registerApnsDevice(deviceToken: string, bundleId?: string): Promise<ApnsRegisterResponse> {
    return this.request('/apns/register', {
      method: 'POST',
      body: JSON.stringify({
        device_token: deviceToken,
        bundle_id: bundleId ?? undefined,
      }),
    });
  }

  async getApnsStatus(): Promise<ApnsStatusResponse> {
    return this.request('/apns/status');
  }

  // Sessions
  async getSessions(): Promise<SessionResponse[]> {
    return this.request('/sessions');
  }

  async getSession(id: string): Promise<SessionWithMessagesResponse> {
    return this.request(`/sessions/${id}`);
  }

  async createSession(
    title?: string,
    projectDir?: string,
    targetBranch?: string,
    workspaceMode?: WorkspaceMode,
    sessionType?: SessionType,
  ): Promise<SessionResponse> {
    return this.request('/sessions', {
      method: 'POST',
      body: JSON.stringify({
        title: title ?? undefined,
        project_dir: projectDir ?? undefined,
        target_branch: targetBranch ?? undefined,
        workspace_mode: workspaceMode ?? undefined,
        session_type: sessionType ?? undefined,
      }),
    });
  }

  async updateSession(id: string, data: Partial<{ title: string; mode: string; model: string }>): Promise<SessionResponse> {
    return this.request(`/sessions/${id}`, {
      method: 'PATCH',
      body: JSON.stringify(data),
    });
  }

  async deleteSession(id: string): Promise<void> {
    await this.request(`/sessions/${id}`, { method: 'DELETE' });
  }

  async getSessionState(id: string): Promise<SessionStateResponse> {
    return this.request(`/sessions/${id}/state`);
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
      method: 'PUT',
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
    return this.request('/models');
  }

  // Git
  async getGitStatus(path?: string): Promise<GitStatusResponse> {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return this.request(`/git/status${q}`);
  }

  async getGitBranches(path?: string): Promise<GitBranchesResponse> {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return this.request(`/git/branches${q}`);
  }

  async getGitWorktrees(path?: string): Promise<GitWorktreesResponse> {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return this.request(`/git/worktrees${q}`);
  }

  async checkoutGitBranch(
    branch: string,
    path?: string,
    create?: boolean,
    startPoint?: string,
  ): Promise<GitStatusResponse> {
    return this.request('/git/checkout', {
      method: 'POST',
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
    return this.request('/credentials');
  }

  async setCredential(providerId: string, apiKey: string): Promise<ProviderStatus> {
    return this.request(`/credentials/${providerId}`, {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
    });
  }

  async deleteCredential(providerId: string): Promise<void> {
    await this.request(`/credentials/${providerId}`, { method: 'DELETE' });
  }

  async startOAuth(provider: string): Promise<OAuthStartResponse> {
    return this.request('/auth/oauth/start', {
      method: 'POST',
      body: JSON.stringify({ provider }),
    });
  }

  async getOAuthStatus(provider: string): Promise<OAuthStatusResponse> {
    return this.request(`/auth/oauth/status/${provider}`);
  }

  async exchangeOAuthCode(provider: string, code: string): Promise<OAuthExchangeResponse> {
    return this.request('/auth/oauth/exchange', {
      method: 'POST',
      body: JSON.stringify({ provider, code }),
    });
  }

  async revokeOAuth(provider: string): Promise<OAuthStatusResponse> {
    return this.request(`/auth/oauth/revoke/${provider}`, { method: 'DELETE' });
  }

  // Files
  async getFileTree(root?: string, depth = 3): Promise<{ root: string; entries: TreeEntry[] }> {
    const params = new URLSearchParams();
    if (root) params.set('root', root);
    params.set('depth', String(depth));
    return this.request(`/files/tree?${params}`);
  }

  async getFile(path: string): Promise<{ path: string; content: string; size: number }> {
    return this.request(`/files?path=${encodeURIComponent(path)}`);
  }

  // Server
  async getServerAccess(): Promise<ServerAccessResponse> {
    return this.request('/server/access');
  }

  // Preview / Ports
  async getPorts(): Promise<PortListResponse> {
    return this.request('/ports');
  }

  async getPreviewSettings(): Promise<PreviewSettings> {
    return this.request('/settings/preview');
  }

  async updatePreviewSettings(patch: PreviewSettingsPatch): Promise<PreviewSettings> {
    return this.request('/settings/preview', {
      method: 'PATCH',
      body: JSON.stringify(patch),
    });
  }

  async addPinnedPort(port: number): Promise<PreviewSettings> {
    return this.request('/settings/preview/pins', {
      method: 'POST',
      body: JSON.stringify({ port }),
    });
  }

  async removePinnedPort(port: number): Promise<PreviewSettings> {
    return this.request(`/settings/preview/pins/${port}`, { method: 'DELETE' });
  }

  async addHiddenPort(port: number): Promise<PreviewSettings> {
    return this.request('/settings/preview/hidden', {
      method: 'POST',
      body: JSON.stringify({ port }),
    });
  }

  async removeHiddenPort(port: number): Promise<PreviewSettings> {
    return this.request(`/settings/preview/hidden/${port}`, { method: 'DELETE' });
  }

  // MCP
  async getMcpServers(): Promise<McpServerResponse[]> {
    return this.request('/mcp');
  }

  async reloadMcpConfig(): Promise<McpServerResponse[]> {
    return this.request('/mcp/reload', { method: 'POST' });
  }

  async connectMcpServer(name: string): Promise<McpServerResponse> {
    return this.request(`/mcp/${encodeURIComponent(name)}/connect`, { method: 'POST' });
  }

  async disconnectMcpServer(name: string): Promise<McpServerResponse> {
    return this.request(`/mcp/${encodeURIComponent(name)}/disconnect`, { method: 'POST' });
  }

  async getMcpServerTools(name: string): Promise<McpToolResponse[]> {
    return this.request(`/mcp/${encodeURIComponent(name)}/tools`);
  }

  // Skills
  async getSkills(scope: 'all' | 'global' = 'all'): Promise<SkillInfo[]> {
    const query = scope === 'global' ? '?scope=global' : '';
    return this.request(`/skills${query}`);
  }

  // Tools
  async submitToolApproval(sessionId: string, toolCallId: string, approved: boolean): Promise<{ status: string }> {
    return this.request('/chat/tool-approval', {
      method: 'POST',
      body: JSON.stringify({ session_id: sessionId, tool_call_id: toolCallId, approved }),
    });
  }

  async submitToolResult(sessionId: string, toolCallId: string, result: string): Promise<void> {
    await this.request('/chat/tool-result', {
      method: 'POST',
      body: JSON.stringify({ session_id: sessionId, tool_call_id: toolCallId, result }),
    });
  }

  // Presence (aliases for backward compat)
  async heartbeatSessionPresence(
    sessionId: string,
    data: { client_id: string; surface: string; capability: string; last_event_sequence?: number | null },
  ): Promise<SessionPresenceResponse> {
    return this.heartbeatPresence(sessionId, data.client_id, data.surface, data.capability, data.last_event_sequence);
  }

  async removeSessionPresence(sessionId: string, clientId: string): Promise<SessionPresenceResponse> {
    return this.request(`/sessions/${sessionId}/presence/${clientId}`, { method: 'DELETE' });
  }

  // Pinch
  async pinchSession(
    sessionId: string,
    hints?: string,
    direction?: string,
  ): Promise<{ session: SessionResponse; summary: string; key_decisions: string[]; pending_tasks: string[] }> {
    return this.request(`/sessions/${sessionId}/pinch`, {
      method: 'POST',
      body: JSON.stringify({ hints, direction }),
    });
  }

  // Directories
  async getDirectories(): Promise<string[]> {
    return this.request('/directories');
  }

  async browseDirectories(path?: string): Promise<{
    current: string;
    parent: string | null;
    directories: Array<{ name: string; path: string }>;
  }> {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return this.request(`/files/browse${q}`);
  }

  // Reports
  async getReports(projectDir?: string): Promise<{ reports: ReportSummary[] }> {
    const q = projectDir ? `?project_dir=${encodeURIComponent(projectDir)}` : '';
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
      method: 'POST',
      body: JSON.stringify({
        memory_type: options?.memoryType ?? undefined,
      }),
    });
  }

  async getMemories(
    projectDir?: string,
    memoryType?: MemoryType,
  ): Promise<{ memories: AgentMemory[] }> {
    const params: string[] = [];
    if (projectDir) {
      params.push(`project_dir=${encodeURIComponent(projectDir)}`);
    }
    if (memoryType) {
      params.push(`memory_type=${encodeURIComponent(memoryType)}`);
    }
    const q = params.join('&');
    return this.request(`/memories${q ? `?${q}` : ''}`);
  }

  async getMemorySnapshot(projectDir?: string): Promise<MemorySnapshotResponse> {
    const q = projectDir ? `?project_dir=${encodeURIComponent(projectDir)}` : '';
    return this.request(`/memories/snapshot${q}`);
  }

  // Mako
  async dispatchMako(task: string, options?: { projectDir?: string; model?: string; startAt?: string; priority?: MakoRunPriority }): Promise<MakoDispatchResponse> {
    return this.request('/mako/dispatch', {
      method: 'POST',
      body: JSON.stringify({
        task,
        project_dir: options?.projectDir ?? undefined,
        model: options?.model ?? undefined,
        start_at: options?.startAt ?? undefined,
        priority: options?.priority ?? undefined,
      }),
    });
  }

  async getMakoCurrent(): Promise<MakoCurrentResponse> {
    return this.request('/mako/current');
  }

  async listMakoSessions(): Promise<MakoSessionSummary[]> {
    return this.request('/mako/sessions');
  }

  async getMakoSessionStatus(id: string): Promise<MakoSessionStatus> {
    return this.request(`/mako/sessions/${id}/status`);
  }

  async sendMakoMessage(id: string, message: string): Promise<SimpleOkResponse> {
    return this.request(`/mako/sessions/${id}/message`, {
      method: 'POST',
      body: JSON.stringify({ message }),
    });
  }

  async scheduleMakoSession(id: string, startAt: string): Promise<SimpleOkResponse> {
    return this.request(`/mako/sessions/${id}/schedule`, {
      method: 'POST',
      body: JSON.stringify({ start_at: startAt }),
    });
  }

  async setMakoSessionPriority(id: string, priority: MakoRunPriority): Promise<SimpleOkResponse> {
    return this.request(`/mako/sessions/${id}/priority`, {
      method: 'POST',
      body: JSON.stringify({ priority }),
    });
  }

  async pauseMakoSession(id: string): Promise<SimpleOkResponse> {
    return this.request(`/mako/sessions/${id}/pause`, { method: 'POST' });
  }

  async resumeMakoSession(id: string): Promise<SimpleOkResponse> {
    return this.request(`/mako/sessions/${id}/resume`, { method: 'POST' });
  }

  async cancelMakoSession(id: string): Promise<void> {
    await this.request(`/mako/sessions/${id}`, { method: 'DELETE' });
  }

  async observeMakoSession(
    id: string,
    callbacks: StreamCallbacks,
    signal?: AbortSignal,
  ): Promise<void> {
    return this.streamSSERequest(`/mako/sessions/${id}/events`, 'GET', undefined, callbacks, signal);
  }

  // ============================================================================
  // Streaming
  // ============================================================================

  async streamChat(
    request: ChatRequest,
    callbacks: StreamCallbacks,
    signal?: AbortSignal,
  ): Promise<void> {
    return this.streamSSERequest('/chat', 'POST', request, callbacks, signal);
  }

  async streamToolResult(
    request: { session_id: string; tool_call_id: string; result: string },
    callbacks: StreamCallbacks,
    signal?: AbortSignal,
  ): Promise<void> {
    return this.streamSSERequest('/chat/tool-result', 'POST', request, callbacks, signal);
  }

  private async streamSSERequest(
    path: string,
    method: 'GET' | 'POST',
    body: object | undefined,
    callbacks: StreamCallbacks,
    signal?: AbortSignal,
  ): Promise<void> {
    const url = `${this.baseUrl}/api${path}`;
    const response = await this.fetchFn(url, {
      method,
      headers: {
        ...this.headers(),
        Accept: 'text/event-stream',
      },
      body: body ? JSON.stringify(body) : undefined,
      signal,
    });

    if (!response.ok) {
      const text = await response.text().catch(() => 'Stream failed');
      callbacks.onError(text);
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

    const activityCheck = setInterval(() => {
      if (Date.now() - lastActivity > STREAM_ACTIVITY_TIMEOUT) {
        callbacks.onError('Stream timeout — no activity for 30s');
        reader.cancel();
        clearInterval(activityCheck);
      }
    }, 5000);

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        lastActivity = Date.now();
        buffer += decoder.decode(value, { stream: true });

        const lines = buffer.split('\n');
        buffer = lines.pop() ?? '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed || trimmed.startsWith(':')) continue;

          if (trimmed.startsWith('data: ')) {
            const data = trimmed.slice(6);
            try {
              const event = JSON.parse(data) as StreamEvent;
              this.handleEvent(event, callbacks);
            } catch {
              // Skip malformed events
            }
          }
        }
      }

      if (buffer.trim()) {
        const trimmed = buffer.trim();
        if (trimmed.startsWith('data: ')) {
          try {
            const event = JSON.parse(trimmed.slice(6)) as StreamEvent;
            this.handleEvent(event, callbacks);
          } catch {
            // Skip malformed events
          }
        }
      }
    } catch (err) {
      if (signal?.aborted) return;
      callbacks.onError(err instanceof Error ? err.message : 'Stream error');
    } finally {
      clearInterval(activityCheck);
    }
  }

  private handleEvent(event: StreamEvent, callbacks: StreamCallbacks): void {
    switch (event.type) {
      case 'text_delta':
      case 'text_delta_with_citations':
        callbacks.onTextDelta(event.delta);
        break;
      case 'thinking_delta':
        callbacks.onThinkingDelta(event.thinking);
        break;
      case 'tool_call_start':
        callbacks.onToolCallStart(event.id, event.name);
        break;
      case 'tool_call_complete':
        callbacks.onToolCallComplete(event.id, event.name, event.arguments);
        break;
      case 'tool_result':
        callbacks.onToolResult(event.id, event.output, event.is_error);
        break;
      case 'tool_output_delta':
        callbacks.onToolOutputDelta(event.id, event.delta);
        break;
      case 'delegated_progress':
        callbacks.onDelegatedProgress?.(event as DelegatedProgressEvent);
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
      case 'title_update':
        callbacks.onTitleUpdate(event.title);
        break;
      case 'finish':
        callbacks.onFinish(event.session_id);
        break;
      case 'error':
        callbacks.onError(event.error);
        break;
      // Mako autonomous agent events
      case 'user_message':
        callbacks.onUserMessage?.(event.title, event.message, event.level);
        break;
      case 'agent_sleeping':
        callbacks.onAgentSleeping?.(event.duration_secs, event.reason);
        break;
      case 'tick_injected':
        callbacks.onTickInjected?.(event.tick_number);
        break;
      case 'classifier_decision':
        callbacks.onClassifierDecision?.(event.tool_name, event.decision, event.reason, event.stage);
        break;
      case 'teammate_spawned':
        callbacks.onTeammateSpawned?.(event.name, event.role);
        break;
      case 'teammate_task_completed':
        callbacks.onTeammateTaskCompleted?.(event.name, event.task_id, event.result);
        break;
      case 'teammate_task_failed':
        callbacks.onTeammateTaskFailed?.(event.name, event.task_id, event.error);
        break;
      case 'teammate_cancelled':
        callbacks.onTeammateCancelled?.(event.name);
        break;
    }
  }
}
