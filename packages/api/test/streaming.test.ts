import { describe, expect, test } from "bun:test";

import {
	type DelegationEventResponse,
	MitsuroClient,
	type ModelKey,
	type StreamCallbacks,
} from "../src";

function streamResponse(...chunks: string[]): Response {
	const encoder = new TextEncoder();
	return byteStreamResponse(chunks.map((chunk) => encoder.encode(chunk)));
}

function byteStreamResponse(chunks: Uint8Array[]): Response {
	return new Response(
		new ReadableStream<Uint8Array>({
			start(controller) {
				for (const chunk of chunks) {
					controller.enqueue(chunk);
				}
				controller.close();
			},
		}),
		{
			status: 200,
			headers: { "Content-Type": "text/event-stream" },
		},
	);
}

function createCallbacks(
	overrides: Partial<StreamCallbacks> = {},
): StreamCallbacks {
	return {
		onTextDelta: () => {},
		onThinkingDelta: () => {},
		onToolCallStart: () => {},
		onToolCallComplete: () => {},
		onToolResult: () => {},
		onToolOutputDelta: () => {},
		onPlanUpdate: () => {},
		onModeChange: () => {},
		onPlanComplete: () => {},
		onUsage: () => {},
		onTitleUpdate: () => {},
		onFinish: () => {},
		onError: () => {},
		...overrides,
	};
}

function clientFor(response: Response): MitsuroClient {
	return new MitsuroClient({
		baseUrl: "https://mitsuro.test",
		fetchImpl: (async () => response) as typeof fetch,
	});
}

describe("MitsuroClient streaming lifecycle", () => {
	test("sends an exact model key when a chat stream starts", async () => {
		const modelKey: ModelKey = {
			provider: "grok",
			model_id: "grok-4.5",
			auth_scope: "oauth",
			api_format: "open_ai_responses",
		};
		let requestBody: unknown;
		const client = new MitsuroClient({
			baseUrl: "https://mitsuro.test",
			fetchImpl: (async (_input, init) => {
				requestBody = JSON.parse(String(init?.body));
				return streamResponse(
					'data: {"type":"finish","session_id":"session-key","stop_reason":"end_turn"}\n\n',
				);
			}) as typeof fetch,
		});

		await client.streamChat(
			{
				message: "Build the project",
				model: modelKey.model_id,
				model_key: modelKey,
			},
			createCallbacks(),
		);

		expect(requestBody).toEqual({
			message: "Build the project",
			model: "grok-4.5",
			model_key: modelKey,
		});
	});

	test("surfaces a readable 402 provider limit error before streaming starts", async () => {
		const errors: string[] = [];
		const client = clientFor(
			new Response(
				JSON.stringify({ error: "Grok Build usage balance exhausted" }),
				{
					status: 402,
					headers: { "content-type": "application/json" },
				},
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({ onError: (error) => errors.push(error) }),
		);

		expect(errors).toEqual(["API 402: Grok Build usage balance exhausted"]);
	});

	test("reports a clean EOF that has no finish or error event", async () => {
		const errors: string[] = [];
		const deltas: string[] = [];
		const client = clientFor(
			streamResponse('data: {"type":"text_delta","delta":"partial"}\n\n'),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onTextDelta: (delta) => deltas.push(delta),
				onError: (error) => errors.push(error),
			}),
		);

		expect(deltas).toEqual(["partial"]);
		expect(errors).toEqual([
			"Stream ended before the server reported completion. Recovering the session state.",
		]);
	});

	test("accepts spec-valid data fields without a space after the colon", async () => {
		const errors: string[] = [];
		const finishes: Array<[string, string | undefined]> = [];
		const client = clientFor(
			streamResponse(
				'data:{"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onFinish: (sessionId, stopReason) =>
					finishes.push([sessionId, stopReason]),
				onError: (error) => errors.push(error),
			}),
		);

		expect(finishes).toEqual([["session-1", "end_turn"]]);
		expect(errors).toEqual([]);
	});

	test("delivers live steering identity for client-side deduplication", async () => {
		const steering: Array<[string | undefined, string]> = [];
		const client = clientFor(
			streamResponse(
				'data: {"type":"steering_injected","pending_id":"steer-1","message":"change direction"}\n\n' +
					'data: {"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onSteeringInjected: (pendingId, message) =>
					steering.push([pendingId, message]),
			}),
		);

		expect(steering).toEqual([["steer-1", "change direction"]]);
	});

	test("delivers bounded tool argument preparation progress", async () => {
		const progress: Array<[string, string, number]> = [];
		const client = clientFor(
			streamResponse(
				'data: {"type":"tool_call_start","id":"patch-1","name":"apply_patch"}\n\n' +
					'data: {"type":"tool_call_preparing","id":"patch-1","name":"apply_patch","received_bytes":8192}\n\n' +
					'data: {"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onToolCallPreparing: (id, name, receivedBytes) =>
					progress.push([id, name, receivedBytes]),
			}),
		);

		expect(progress).toEqual([["patch-1", "apply_patch", 8192]]);
	});

	test("delivers typed durable delegation events with their replay cursor", async () => {
		const events: unknown[] = [];
		const client = clientFor(
			streamResponse(
				'data: {"type":"delegation_event","event":{"event_id":42,"parent_session_id":"session-1","delegation_group_id":"group-1","delegation_task_id":"task-1","event_type":"task_running","payload":{"state":"running"},"created_at":"2026-08-08T12:00:00Z"}}\n\n' +
					'data: {"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onDelegationEvent: (event) => events.push(event),
			}),
		);

		expect(events).toEqual([
			{
				event_id: 42,
				parent_session_id: "session-1",
				delegation_group_id: "group-1",
				delegation_task_id: "task-1",
				event_type: "task_running",
				payload: { state: "running" },
				created_at: "2026-08-08T12:00:00Z",
			},
		]);
	});

	test("scopes delegation event subscriptions to mounted consumers", async () => {
		const subscribed: DelegationEventResponse[] = [];
		const client = clientFor(
			streamResponse(
				'data: {"type":"delegation_event","event":{"event_id":42,"parent_session_id":"session-1","delegation_group_id":"group-1","delegation_task_id":"task-1","event_type":"task_running","payload":{"state":"running"},"created_at":"2026-08-08T12:00:00Z"}}\n\n' +
					'data: {"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);
		const unsubscribe = client.subscribeDelegationEvents((event) =>
			subscribed.push(event),
		);

		await client.streamChat({ message: "hello" }, createCallbacks());
		unsubscribe();

		expect(subscribed.map((event) => event.event_id)).toEqual([42]);
	});

	test("preserves unknown durable delegation event kinds", async () => {
		const events: DelegationEventResponse[] = [];
		const futureEventType: DelegationEventResponse["event_type"] =
			"future_scheduler_event";
		const client = clientFor(
			streamResponse(
				'data: {"type":"delegation_event","event":{"event_id":43,"parent_session_id":"session-1","delegation_group_id":"group-1","delegation_task_id":null,"event_type":"future_scheduler_event","payload":{"domain":"workspace"},"created_at":"2026-08-08T12:00:01Z"}}\n\n' +
					'data: {"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onDelegationEvent: (event) => events.push(event),
			}),
		);

		expect(events[0]?.event_type).toBe(futureEventType);
		expect(events[0]?.payload).toEqual({ domain: "workspace" });
	});

	test("does not add a premature-close error after an explicit server error", async () => {
		const errors: string[] = [];
		const client = clientFor(
			streamResponse('data: {"type":"error","error":"balance exhausted"}\n\n'),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({ onError: (error) => errors.push(error) }),
		);

		expect(errors).toEqual(["balance exhausted"]);
	});

	test("preserves split UTF-8 code points and CRLF event boundaries", async () => {
		const encoder = new TextEncoder();
		const payload = encoder.encode(
			'data: {"type":"text_delta","delta":"working 🦀"}\r\n\r\n' +
				'data: {"type":"finish","session_id":"session-utf8","stop_reason":"end_turn"}\r\n\r\n',
		);
		const emojiStart = payload.indexOf(0xf0);
		expect(emojiStart).toBeGreaterThan(0);
		const client = clientFor(
			byteStreamResponse([
				payload.slice(0, emojiStart + 2),
				payload.slice(emojiStart + 2),
			]),
		);
		const deltas: string[] = [];
		const finishes: string[] = [];
		const errors: string[] = [];

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onTextDelta: (delta) => deltas.push(delta),
				onFinish: (sessionId) => finishes.push(sessionId),
				onError: (error) => errors.push(error),
			}),
		);

		expect(deltas).toEqual(["working 🦀"]);
		expect(finishes).toEqual(["session-utf8"]);
		expect(errors).toEqual([]);
	});

	test("joins multiline data fields and ignores comments and unknown fields", async () => {
		const deltas: string[] = [];
		const errors: string[] = [];
		const client = clientFor(
			streamResponse(
				': heartbeat\r\nevent: message\r\ndata: {"type":"text_delta",\r\ndata: "delta":"joined"}\r\n\r\n' +
					'data: {"type":"finish","session_id":"session-2","stop_reason":"end_turn"}\r\n\r\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onTextDelta: (delta) => deltas.push(delta),
				onError: (error) => errors.push(error),
			}),
		);

		expect(deltas).toEqual(["joined"]);
		expect(errors).toEqual([]);
	});

	test("isolates malformed records so a following finish still completes", async () => {
		const finishes: string[] = [];
		const errors: string[] = [];
		const client = clientFor(
			streamResponse(
				'data: {not-json}\n\ndata: {"type":"finish","session_id":"session-3","stop_reason":"end_turn"}\n\n',
			),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onFinish: (sessionId) => finishes.push(sessionId),
				onError: (error) => errors.push(error),
			}),
		);

		expect(finishes).toEqual(["session-3"]);
		expect(errors).toEqual([]);
	});

	test("an intentional abort does not surface as a stream error", async () => {
		const controller = new AbortController();
		controller.abort();
		const errors: string[] = [];
		const client = new MitsuroClient({
			baseUrl: "https://mitsuro.test",
			fetchImpl: (async () => {
				throw new DOMException("Aborted", "AbortError");
			}) as typeof fetch,
		});

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({ onError: (error) => errors.push(error) }),
			controller.signal,
		);

		expect(errors).toEqual([]);
	});
});
