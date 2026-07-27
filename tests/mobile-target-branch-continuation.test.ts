import assert from "node:assert/strict";
import test from "node:test";

import {
  KrustyClient,
  type ChatRequest,
  type SessionResponse,
  type SessionStateResponse,
  type SessionWithMessagesResponse,
  type StreamCallbacks,
} from "../packages/api/src";
import {
  MemoryStorage,
  createPlanStore,
  createSessionStore,
  createSessionsStore,
  createWorkspaceStore,
} from "../packages/state/src";
import {
  findCodeSessionForProject,
  resolveSendIntent,
} from "../apps/mobile/app/(tabs)/chat-screen/sendIntent";

function makeSession(overrides: Partial<SessionResponse> = {}): SessionResponse {
  return {
    id: "session-1",
    title: "Session 1",
    token_count: 0,
    working_dir: null,
    project_dir: null,
    workspace_mode: "neutral",
    session_type: "code",
    parent_session_id: null,
    mode: "build",
    updated_at: "2026-05-08T00:00:00Z",
    model: null,
    target_branch: null,
    ...overrides,
  };
}

function makeSessionState(id: string): SessionStateResponse {
  return {
    id,
    agent_state: "idle",
    started_at: null,
    last_event_at: null,
    mode: "build",
    recovery: null,
    live_partial_assistant: null,
    delegated_tools: [],
    recent_delegated_runs: [],
    last_event_sequence: null,
  };
}

function jsonResponse(data: unknown): Response {
  return new Response(JSON.stringify(data), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function makeCapturingFetch(captures: Array<{ url: string; init: RequestInit }>) {
  return (async (url: RequestInfo | URL, init?: RequestInit) => {
    captures.push({ url: String(url), init: init ?? {} });
    const body = init?.body ? JSON.parse(String(init.body)) : {};
    return jsonResponse(makeSession({
      id: "api-created-session",
      project_dir: body.project_dir ?? null,
      working_dir: body.project_dir ?? null,
      workspace_mode: body.workspace_mode ?? "selected",
      session_type: body.session_type ?? "code",
      target_branch: body.target_branch ?? null,
    }));
  }) as typeof fetch;
}

function makeHarness(initialSession: SessionResponse = makeSession()) {
  const storage = new MemoryStorage();
  const workspace = createWorkspaceStore(storage);
  let capturedChatRequest: ChatRequest | null = null;
  const createSessionCalls: Array<{
    title?: string;
    projectDir?: string;
    targetBranch?: string;
    workspaceMode?: string;
    sessionType?: string;
  }> = [];

  const client = {
    async getSessions(): Promise<SessionResponse[]> {
      return [initialSession];
    },
    async getDirectories(): Promise<string[]> {
      return [];
    },
    async createSession(
      title?: string,
      projectDir?: string,
      targetBranch?: string,
      workspaceMode?: string,
      sessionType?: string,
    ): Promise<SessionResponse> {
      createSessionCalls.push({
        title,
        projectDir,
        targetBranch,
        workspaceMode,
        sessionType,
      });
      return makeSession({
        id: "created-session",
        title: title ?? "New Session",
        project_dir: projectDir ?? null,
        working_dir: projectDir ?? null,
        workspace_mode: (workspaceMode as SessionResponse["workspace_mode"]) ?? (projectDir ? "selected" : "neutral"),
        session_type: (sessionType as SessionResponse["session_type"]) ?? "code",
        target_branch: targetBranch ?? null,
      });
    },
    async deleteSession(): Promise<void> {},
    async getSession(id: string): Promise<SessionWithMessagesResponse> {
      return { session: makeSession({ ...initialSession, id }), messages: [] };
    },
    async getSessionState(id: string): Promise<SessionStateResponse> {
      return makeSessionState(id);
    },
    async updateSession(id: string, data: Record<string, unknown>): Promise<SessionResponse> {
      return makeSession({ ...initialSession, id, ...data });
    },
    async heartbeatPresence(): Promise<unknown> {
      return {};
    },
    async removeSessionPresence(): Promise<void> {},
    async setCurrentModel(): Promise<unknown> {
      return { ok: true };
    },
    async streamChat(
      request: ChatRequest,
      callbacks: StreamCallbacks,
    ): Promise<void> {
      capturedChatRequest = request;
      callbacks.onFinish("created-session");
    },
  };

  const sessionsStore = createSessionsStore(client as never, workspace);
  const sessionStore = createSessionStore(
    client as never,
    storage,
    workspace,
    sessionsStore,
    createPlanStore(),
  );

  return {
    storage,
    workspace,
    sessionsStore,
    sessionStore,
    createSessionCalls,
    getCapturedChatRequest: () => capturedChatRequest,
  };
}

test("API client createSession serializes target_branch for project opens", async () => {
  const captures: Array<{ url: string; init: RequestInit }> = [];
  const client = new KrustyClient({
    baseUrl: "https://krusty.invalid",
    fetchImpl: makeCapturingFetch(captures),
  });

  const session = await client.createSession(
    undefined,
    "/repo/app",
    "feature/project-open",
    "selected",
    "code",
  );

  const request = captures[0];
  assert.equal(request?.url, "https://krusty.invalid/api/sessions");
  assert.equal(request?.init.method, "POST");
  assert.deepEqual(JSON.parse(String(request?.init.body)), {
    project_dir: "/repo/app",
    target_branch: "feature/project-open",
    workspace_mode: "selected",
    session_type: "code",
  });
  assert.equal(session.target_branch, "feature/project-open");
});

test("API client updateSession serializes explicit target_branch clear", async () => {
  const captures: Array<{ url: string; init: RequestInit }> = [];
  const client = new KrustyClient({
    baseUrl: "https://krusty.invalid/",
    fetchImpl: makeCapturingFetch(captures),
  });

  await client.updateSession("session-1", { target_branch: null });

  const request = captures[0];
  assert.equal(request?.url, "https://krusty.invalid/api/sessions/session-1");
  assert.equal(request?.init.method, "PATCH");
  assert.deepEqual(JSON.parse(String(request?.init.body)), {
    target_branch: null,
  });
});

test("first-send code chat request preserves selected targetBranch intent", async () => {
  const { workspace, sessionStore, getCapturedChatRequest } = makeHarness();

  workspace
    .getState()
    .setWorkspace("/repo/app", null, "selected", "feature/mobile-continuation");

  await sessionStore.getState().sendMessage("continue work", [], {
    sessionType: "code",
  });

  const request = getCapturedChatRequest();
  assert.equal(request?.project_dir, "/repo/app");
  assert.equal(request?.working_dir, "/repo/app");
  assert.equal(request?.workspace_mode, "selected");
  assert.equal(request?.session_type, "code");
  assert.equal(request?.target_branch, "feature/mobile-continuation");
});

test("explicit projectDir send option also becomes workingDir when workingDir is omitted", async () => {
  const { workspace, sessionStore, getCapturedChatRequest } = makeHarness();

  workspace
    .getState()
    .setWorkspace("/repo/stale", null, "selected", "feature/stale");

  await sessionStore.getState().sendMessage("open target", [], {
    projectDir: "/repo/target",
    workspaceMode: "selected",
    sessionType: "code",
    targetBranch: "feature/target",
  });

  const request = getCapturedChatRequest();
  assert.equal(request?.project_dir, "/repo/target");
  assert.equal(request?.working_dir, "/repo/target");
  assert.equal(request?.workspace_mode, "selected");
  assert.equal(request?.session_type, "code");
  assert.equal(request?.target_branch, "feature/target");
});

test("loading a session snapshot persists targetBranch in workspace state", async () => {
  const initialSession = makeSession({
    id: "mako-run-1",
    session_type: "mako",
    project_dir: "/repo/app",
    working_dir: "/repo/app",
    workspace_mode: "selected",
    target_branch: "feature/mako-run",
  });
  const { storage, workspace, sessionStore } = makeHarness(initialSession);

  try {
    await sessionStore.getState().loadSession("mako-run-1");

    assert.equal(workspace.getState().directory, "/repo/app");
    assert.equal(workspace.getState().targetBranch, "feature/mako-run");
    assert.equal(
      JSON.parse(storage.get("krusty:workspace") ?? "{}").targetBranch,
      "feature/mako-run",
    );
  } finally {
    sessionStore.getState().cleanup();
  }
});

test("creating a project-scoped session persists targetBranch in workspace state", async () => {
  const { workspace, sessionsStore, createSessionCalls } = makeHarness();

  const session = await sessionsStore
    .getState()
    .createSession(undefined, "/repo/app", "feature/project-open");

  assert.equal(session?.target_branch, "feature/project-open");
  assert.equal(createSessionCalls[0]?.targetBranch, "feature/project-open");
  assert.equal(workspace.getState().directory, "/repo/app");
  assert.equal(workspace.getState().targetBranch, "feature/project-open");
});

test("project open lookup scopes code sessions by projectDir and targetBranch", () => {
  const sessions = [
    makeSession({
      id: "neutral-branch-session",
      project_dir: "/repo/app",
      working_dir: "/repo/app",
      target_branch: null,
    }),
    makeSession({
      id: "feature-branch-session",
      project_dir: "/repo/app",
      working_dir: "/repo/app",
      target_branch: "feature/project-open",
    }),
    makeSession({
      id: "other-project-session",
      project_dir: "/repo/other",
      working_dir: "/repo/other",
      target_branch: "feature/project-open",
    }),
  ];

  assert.equal(
    findCodeSessionForProject(sessions, "/repo/app", "feature/project-open")?.id,
    "feature-branch-session",
  );
  assert.equal(
    findCodeSessionForProject(sessions, "/repo/app", null)?.id,
    "neutral-branch-session",
  );
  assert.equal(
    findCodeSessionForProject(sessions, "/repo/app", "feature/missing"),
    null,
  );
});

test("code first-send intent streams selected project and targetBranch without neutral precreate", () => {
  const intent = resolveSendIntent({
    activeTab: 1,
    currentSessionId: null,
    workspaceDirectory: "/repo/app",
    workspaceMode: "selected",
    targetBranch: "feature/mobile-continuation",
  });

  assert.equal(intent.shouldPrecreate, false);
  assert.deepEqual(intent.sendOptions, {
    projectDir: "/repo/app",
    workingDir: "/repo/app",
    workspaceMode: "selected",
    sessionType: "code",
    targetBranch: "feature/mobile-continuation",
  });
});

test("chat first-send keeps explicit neutral no-branch precreate even with stale code workspace", () => {
  const intent = resolveSendIntent({
    activeTab: 0,
    currentSessionId: null,
    workspaceDirectory: "/repo/app",
    workspaceMode: "selected",
    targetBranch: "feature/should-not-leak",
  });

  assert.equal(intent.shouldPrecreate, true);
  assert.deepEqual(intent.precreate, {
    workspaceMode: "neutral",
    sessionType: "chat",
  });
  assert.equal(intent.sendOptions, undefined);
});
