import { describe, expect, it } from "bun:test";

import {
	KrustyApiError,
	KrustyClient,
	type ChatRequest,
	type ModelKey,
	type ModelsResponse,
} from "../src";

const exactGrokKey: ModelKey = {
	provider: "grok",
	model_id: "grok-4.5",
	auth_scope: "oauth",
	api_format: "open_ai_responses",
};

describe("KrustyClient request errors", () => {
	it("preserves HTTP status and response body in a typed error", async () => {
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () =>
				new Response(JSON.stringify({ error: "Session missing not found" }), {
					status: 404,
					headers: { "content-type": "application/json" },
				}),
		});

		try {
			await client.getSession("missing");
			throw new Error("expected request to fail");
		} catch (error) {
			expect(error).toBeInstanceOf(KrustyApiError);
			const apiError = error as KrustyApiError;
			expect(apiError.status).toBe(404);
			expect(apiError.responseBody).toContain("Session missing not found");
			expect(apiError.message).toBe("API 404: Session missing not found");
		}
	});

	it("preserves a 402 limit response for the visible provider error path", async () => {
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () =>
				new Response(
					JSON.stringify({ error: "Grok Build usage balance exhausted" }),
					{
						status: 402,
						headers: { "content-type": "application/json" },
					},
				),
		});

		await expect(client.getSession("limited")).rejects.toMatchObject({
			status: 402,
			message: "API 402: Grok Build usage balance exhausted",
		});
	});
});

describe("KrustyClient content-free request diagnostics", () => {
	it("requests delegated history only for full session hydration", async () => {
		const urls: string[] = [];
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async (input) => {
				urls.push(String(input));
				return Response.json({});
			},
		});

		await client.getSessionState("session-id");
		await client.getSessionState("session-id", {
			includeDelegatedHistory: true,
		});

		expect(new URL(urls[0] as string).search).toBe("");
		expect(new URL(urls[1] as string).searchParams.get("include_delegated_history"))
			.toBe("true");
	});

	it("reports a sanitized route family and terminal timing", async () => {
		const events: Array<{
			name: string;
			outcome: string;
			durationMs?: number;
			code?: string;
		}> = [];
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () => Response.json({
				id: "session-private-id",
				messages: [],
			}),
			requestObserver: (event) => events.push(event),
		});

		await client.getSession("session-private-id");

		expect(events.map(({ name, outcome, code }) => ({ name, outcome, code })))
			.toEqual([
				{ name: "api.sessions.detail", outcome: "start", code: undefined },
				{ name: "api.sessions.detail", outcome: "complete", code: "http.2xx" },
			]);
		expect(events[1]?.durationMs).toBeGreaterThanOrEqual(0);
		expect(JSON.stringify(events)).not.toContain("session-private-id");
	});

	it("separates privacy-safe session request families without identifiers", async () => {
		const names: string[] = [];
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async (input) => {
				const path = new URL(String(input)).pathname;
				if (path === "/sessions") return Response.json([]);
				if (path === "/sessions/directories") return Response.json([]);
				return Response.json({});
			},
			requestObserver: (event) => {
				if (event.outcome === "start") names.push(event.name);
			},
		});

		await client.getSessions();
		await client.getSessionState("private-session-id");
		await client.getWorkflow("private-session-id");
		await client.getSessionPresence("private-session-id");
		await client.pinchSession("private-session-id");
		await client.getDirectories();

		expect(names).toEqual([
			"api.sessions.catalog",
			"api.sessions.state",
			"api.sessions.workflow",
			"api.sessions.presence",
			"api.sessions.action",
			"api.sessions.directories",
		]);
		expect(JSON.stringify(names)).not.toContain("private-session-id");
	});

	it("separates HTTP and transport failures without response content", async () => {
		const httpEvents: Array<{ outcome: string; code?: string }> = [];
		const httpClient = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () => new Response("private provider detail", { status: 503 }),
			requestObserver: (event) => httpEvents.push(event),
		});
		await expect(httpClient.getModels()).rejects.toBeInstanceOf(KrustyApiError);

		const networkEvents: Array<{ outcome: string; code?: string }> = [];
		const networkClient = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () => {
				throw new TypeError("private network detail");
			},
			requestObserver: (event) => networkEvents.push(event),
		});
		await expect(networkClient.getModels()).rejects.toBeInstanceOf(TypeError);

		expect(httpEvents.at(-1)).toMatchObject({
			outcome: "error",
			code: "http.5xx",
		});
		expect(networkEvents.at(-1)).toMatchObject({
			outcome: "error",
			code: "network.error",
		});
		expect(JSON.stringify([...httpEvents, ...networkEvents]))
			.not.toContain("private");
	});

	it("uses the configured native fetch implementation for health checks", async () => {
		let requestUrl = "";
		const events: string[] = [];
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async (input) => {
				requestUrl = String(input);
				return new Response(null, { status: 204 });
			},
			requestObserver: (event) =>
				events.push(`${event.name}:${event.outcome}:${event.code ?? ""}`),
		});

		expect(await client.checkHealth()).toBe(true);
		expect(requestUrl).toBe("http://krusty.test/health");
		expect(events).toEqual([
			"api.health:start:",
			"api.health:complete:http.2xx",
		]);
	});

	it("cannot let a diagnostics observer change request behavior", async () => {
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: async () => Response.json({
				models: [],
				default_model: null,
			}),
			requestObserver: () => {
				throw new Error("observer failure");
			},
		});

		expect((await client.getModels()).models).toEqual([]);
	});
});

describe("KrustyClient provider-aware model identity", () => {
	it("sends the exact key and legacy model mirror when dispatching Mako", async () => {
		let requestUrl = "";
		let requestInit: RequestInit | undefined;
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: (async (input, init) => {
				requestUrl = String(input);
				requestInit = init;
				return Response.json({ session_id: "mako-1", status: "started" });
			}) as typeof fetch,
		});

		await client.dispatchMako("Audit this project", {
			model: exactGrokKey.model_id,
			modelKey: exactGrokKey,
			projectDir: "/work/project",
		});

		expect(requestUrl).toBe("http://krusty.test/api/mako/dispatch");
		expect(requestInit?.method).toBe("POST");
		expect(JSON.parse(String(requestInit?.body))).toEqual({
			task: "Audit this project",
			project_dir: "/work/project",
			model: "grok-4.5",
			model_key: exactGrokKey,
		});
	});

	it("keeps legacy Mako dispatches compatible by omitting model_key", async () => {
		let body: unknown;
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: (async (_input, init) => {
				body = JSON.parse(String(init?.body));
				return Response.json({ session_id: "mako-legacy", status: "started" });
			}) as typeof fetch,
		});

		await client.dispatchMako("Continue", { model: "grok-4.5" });

		expect(body).toEqual({ task: "Continue", model: "grok-4.5" });
	});

	it("persists and reads an exact default model selection", async () => {
		const requests: Array<{ url: string; body?: unknown }> = [];
		const response: ModelsResponse = {
			models: [],
			default_model: exactGrokKey.model_id,
			default_model_key: exactGrokKey,
		};
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: (async (input, init) => {
				requests.push({
					url: String(input),
					body: init?.body ? JSON.parse(String(init.body)) : undefined,
				});
				return String(input).endsWith("/models")
					? Response.json(response)
					: Response.json({ ok: true });
			}) as typeof fetch,
		});

		await client.setCurrentModel(exactGrokKey.model_id, exactGrokKey);
		const models = await client.getModels();

		expect(requests[0]).toEqual({
			url: "http://krusty.test/api/models/current",
			body: { model: "grok-4.5", model_key: exactGrokKey },
		});
		expect(models.default_model_key).toEqual(exactGrokKey);
	});

	it("types a chat start with the same exact model identity", () => {
		const request: ChatRequest = {
			message: "Build the project",
			model: exactGrokKey.model_id,
			model_key: exactGrokKey,
			allowed_tools: ["read", "grep"],
		};

		expect(request.model_key).toEqual(exactGrokKey);
		expect(request.allowed_tools).toEqual(["read", "grep"]);
	});
});

describe("KrustyClient Mako schedules", () => {
	it("uses a strong quoted revision for schedule status mutations", async () => {
		let requestInit: RequestInit | undefined;
		const client = new KrustyClient({
			baseUrl: "http://krusty.test",
			fetchImpl: (async (_input, init) => {
				requestInit = init;
				return Response.json({
					schedule_id: "schedule-1",
					revision: 8,
					status: "paused",
				});
			}) as typeof fetch,
		});

		await client.pauseMakoSchedule("session-1", "schedule-1", 7);

		expect(requestInit?.method).toBe("POST");
		expect((requestInit?.headers as Record<string, string>)["If-Match"]).toBe(
			'"7"',
		);
	});
});
