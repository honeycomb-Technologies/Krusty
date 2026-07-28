import { createSessionStore } from "../src/session/store.ts";

declare const Deno: {
	test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
	if (!condition) {
		throw new Error(message);
	}
}

function assertEquals<T>(actual: T, expected: T, message: string) {
	if (!Object.is(actual, expected)) {
		throw new Error(
			`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
		);
	}
}

function createStorage() {
	const data = new Map<string, string>();
	return {
		get: (key: string) => data.get(key) ?? null,
		set: (key: string, value: string) => {
			data.set(key, value);
		},
		delete: (key: string) => {
			data.delete(key);
		},
	};
}

function createWorkspace() {
	let directory: string | null = null;
	let sessionId: string | null = null;
	let mode: "neutral" | "selected" | "created" = "neutral";
	let targetBranch: string | null = null;
	return {
		getState: () => ({
			directory,
			mode,
			sessionId,
			targetBranch,
			initFromSession: (
				nextSessionId: string,
				nextDirectory: string | null,
				nextMode: "neutral" | "selected" | "created" = "neutral",
				nextTargetBranch: string | null = null,
			) => {
				sessionId = nextSessionId;
				directory = nextDirectory;
				mode = nextMode;
				targetBranch = nextTargetBranch;
			},
			setSession: (nextSessionId: string | null) => {
				sessionId = nextSessionId;
			},
			clear: () => {
				sessionId = null;
				directory = null;
				mode = "neutral";
				targetBranch = null;
			},
		}),
	};
}

function createSessionsStore(sessions: Array<Record<string, unknown>> = []) {
	return {
		getState: () => ({
			sessions,
			loadSessions: () => {},
		}),
	};
}

function createPlanStore() {
	return {
		getState: () => ({
			setVisible: () => {},
			setItems: () => {},
			setWorkflow: () => {},
		}),
	};
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

function sessionResponse(
	id: string,
	title: string,
	messageText: string,
) {
	return {
		session: {
			id,
			title,
			token_count: 12,
			working_dir: `/work/${id}`,
			project_dir: `/work/${id}`,
			workspace_mode: "selected",
			session_type: "chat",
			parent_session_id: null,
			mode: "build",
			updated_at: new Date().toISOString(),
			model: "gpt-test",
			model_key: null,
			target_branch: null,
			permission_mode: "autonomous",
		},
		messages: [
			{
				role: "user",
				content: messageText,
			},
		],
	};
}

Deno.test("loadSession activates the destination shell before the network resolves", async () => {
	const first = deferred<ReturnType<typeof sessionResponse>>();
	const second = deferred<ReturnType<typeof sessionResponse>>();
	const requests: string[] = [];

	const client = {
		getSession: async (sessionId: string) => {
			requests.push(sessionId);
			if (sessionId === "session-a") {
				return first.promise;
			}
			if (sessionId === "session-b") {
				return second.promise;
			}
			throw new Error(`unexpected session ${sessionId}`);
		},
		getSessionState: async () => ({
			id: "ignored",
			agent_state: "idle",
			started_at: null,
			last_event_at: null,
			mode: "build",
			permission_mode: "autonomous",
			recovery: null,
			live_partial_assistant: null,
			pending_interactions: [],
			delegated_tools: [],
			recent_delegated_runs: [],
			last_event_sequence: null,
		}),
		heartbeatSessionPresence: async () => ({}),
		removeSessionPresence: async () => ({}),
		updateSession: async () => ({}),
		setCurrentModel: async () => ({}),
	};

	const store = createSessionStore(
		client as never,
		createStorage(),
		createWorkspace() as never,
		createSessionsStore([
			{
				id: "session-a",
				title: "Alpha",
				session_type: "chat",
				project_dir: "/work/session-a",
				working_dir: "/work/session-a",
				workspace_mode: "selected",
				mode: "build",
				permission_mode: "autonomous",
				model: "gpt-test",
				token_count: 1,
			},
			{
				id: "session-b",
				title: "Beta",
				session_type: "chat",
				project_dir: "/work/session-b",
				working_dir: "/work/session-b",
				workspace_mode: "selected",
				mode: "build",
				permission_mode: "autonomous",
				model: "gpt-test",
				token_count: 2,
			},
		]) as never,
		createPlanStore() as never,
	);

	const loadA = store.getState().loadSession("session-a");
	assertEquals(store.getState().sessionId, "session-a", "session A shell should activate immediately");
	assertEquals(store.getState().title, "Alpha", "list metadata should populate the shell title");
	assertEquals(store.getState().isLoading, true, "cold load should show progressive loading");
	assertEquals(store.getState().messages.length, 0, "cold load has no cached messages yet");

	first.resolve(sessionResponse("session-a", "Alpha", "hello from a"));
	await loadA;
	assertEquals(store.getState().isLoading, false, "session A finishes loading");
	assertEquals(store.getState().messages.length > 0, true, "session A messages hydrate");

	const loadB = store.getState().loadSession("session-b");
	assertEquals(store.getState().sessionId, "session-b", "session B shell should activate immediately");
	assertEquals(store.getState().title, "Beta", "session B title should not wait for network");
	assertEquals(store.getState().isLoading, true, "first open of B is still progressive");

	// Returning to A should paint cached messages instantly while refresh continues.
	const reloadA = store.getState().loadSession("session-a");
	assertEquals(store.getState().sessionId, "session-a", "return navigation should flip immediately");
	assertEquals(
		store.getState().messages.some((message) => message.content.includes("hello from a")),
		true,
		"cached transcript should paint before the network returns",
	);
	assertEquals(store.getState().isLoading, false, "cached navigation should not block on a spinner");

	second.resolve(sessionResponse("session-b", "Beta", "hello from b"));
	await Promise.allSettled([loadB, reloadA]);

	assert(
		requests.includes("session-a") && requests.includes("session-b"),
		"both sessions should still refresh from the server",
	);
	store.getState().cleanup();
});

Deno.test("unresolved A to B to A navigation honors the latest selection intent", async () => {
	const firstA = deferred<ReturnType<typeof sessionResponse>>();
	const sessionB = deferred<ReturnType<typeof sessionResponse>>();
	let aRequests = 0;

	const client = {
		getSession: (sessionId: string) => {
			if (sessionId === "session-a") {
				aRequests += 1;
				return firstA.promise;
			}
			if (sessionId === "session-b") return sessionB.promise;
			throw new Error(`unexpected session ${sessionId}`);
		},
		getSessionState: async () => ({
			id: "ignored",
			agent_state: "idle",
			started_at: null,
			last_event_at: null,
			mode: "build",
			permission_mode: "autonomous",
			recovery: null,
			live_partial_assistant: null,
			pending_interactions: [],
			delegated_tools: [],
			recent_delegated_runs: [],
			last_event_sequence: null,
		}),
		heartbeatSessionPresence: async () => ({}),
		removeSessionPresence: async () => ({}),
		updateSession: async () => ({}),
		setCurrentModel: async () => ({}),
	};

	const store = createSessionStore(
		client as never,
		createStorage(),
		createWorkspace() as never,
		createSessionsStore([
			{ id: "session-a", title: "Alpha", session_type: "chat", mode: "build", permission_mode: "autonomous" },
			{ id: "session-b", title: "Beta", session_type: "chat", mode: "build", permission_mode: "autonomous" },
		]) as never,
		createPlanStore() as never,
	);

	const loadFirstA = store.getState().loadSession("session-a");
	const loadB = store.getState().loadSession("session-b");
	const loadLatestA = store.getState().loadSession("session-a");

	assertEquals(store.getState().sessionId, "session-a", "latest selection should activate A immediately");
	assertEquals(aRequests, 1, "returning to unresolved A should reuse its exact hydration request");

	firstA.resolve(sessionResponse("session-a", "Stale Alpha", "stale a"));
	sessionB.resolve(sessionResponse("session-b", "Beta", "stale b"));
	await Promise.all([loadFirstA, loadB, loadLatestA]);
	assertEquals(store.getState().sessionId, "session-a", "stale requests must not replace latest A shell");
	assertEquals(store.getState().title, "Stale Alpha", "shared A response should hydrate the latest A intent");
	assertEquals(
		store.getState().messages.some((message) => message.content.includes("stale a")),
		true,
		"latest A intent should apply the shared transcript once",
	);
	store.getState().cleanup();
});

Deno.test("presence teardown is owned, idempotent, and not resurrected by hidden hydration", async () => {
	const hydration = deferred<ReturnType<typeof sessionResponse>>();
	const heartbeats: string[] = [];
	const removals: string[] = [];
	const client = {
		getSession: () => hydration.promise,
		getSessionState: async () => ({
			id: "session-a",
			agent_state: "idle",
			started_at: null,
			last_event_at: null,
			mode: "build",
			permission_mode: "autonomous",
			recovery: null,
			live_partial_assistant: null,
			pending_interactions: [],
			delegated_tools: [],
			recent_delegated_runs: [],
			last_event_sequence: null,
		}),
		heartbeatSessionPresence: async (sessionId: string) => {
			heartbeats.push(sessionId);
			return {};
		},
		removeSessionPresence: async (sessionId: string) => {
			removals.push(sessionId);
			return {};
		},
		updateSession: async () => ({}),
		setCurrentModel: async () => ({}),
	};
	const store = createSessionStore(
		client as never,
		createStorage(),
		createWorkspace() as never,
		createSessionsStore([
			{ id: "session-a", title: "Alpha", session_type: "chat", mode: "build", permission_mode: "autonomous" },
		]) as never,
		createPlanStore() as never,
	);

	store.getState().startPresenceHeartbeat("owned");
	store.getState().startPresenceHeartbeat("owned");
	await Promise.resolve();
	assertEquals(
		heartbeats.filter((id) => id === "owned").length,
		1,
		"starting the same owned heartbeat twice must be idempotent",
	);
	store.getState().stopPresenceHeartbeat("not-owned");
	await Promise.resolve();
	assertEquals(removals.length, 0, "a mismatched stop must not remove owned presence");

	const load = store.getState().loadSession("session-a");
	store.getState().stopPresenceHeartbeat("session-a");
	hydration.resolve(sessionResponse("session-a", "Alpha", "hydrated"));
	await load;
	await Promise.resolve();
	assertEquals(
		heartbeats.filter((id) => id === "session-a").length,
		1,
		"late hydration must not restart presence after the mode was hidden",
	);
	assertEquals(
		removals.filter((id) => id === "session-a").length,
		1,
		"owned presence should be removed exactly once",
	);
	store.getState().stopPresenceHeartbeat("session-a");
	await Promise.resolve();
	assertEquals(
		removals.filter((id) => id === "session-a").length,
		1,
		"repeated hidden teardown must not issue another remove",
	);
	store.getState().initSession(
		"late-created",
		"Late Created",
		"autonomous",
		"chat",
	);
	await Promise.resolve();
	assertEquals(
		heartbeats.filter((id) => id === "late-created").length,
		0,
		"a late hidden creation bind must not resurrect presence",
	);
	store.getState().cleanup();
});
