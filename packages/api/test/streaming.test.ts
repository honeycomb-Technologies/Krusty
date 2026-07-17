import { describe, expect, test } from "bun:test";

import { KrustyClient, type StreamCallbacks } from "../src";

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

function clientFor(response: Response): KrustyClient {
	return new KrustyClient({
		baseUrl: "https://krusty.test",
		fetchImpl: (async () => response) as typeof fetch,
	});
}

describe("KrustyClient streaming lifecycle", () => {
	test("surfaces a readable 402 provider limit error before streaming starts", async () => {
		const errors: string[] = [];
		const client = clientFor(
			new Response(JSON.stringify({ error: "Grok Build usage balance exhausted" }), {
				status: 402,
				headers: { "content-type": "application/json" },
			}),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({ onError: (error) => errors.push(error) }),
		);

		expect(errors).toEqual([
			"API 402: Grok Build usage balance exhausted",
		]);
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
		const finishes: string[] = [];
		const client = clientFor(
			streamResponse('data:{"type":"finish","session_id":"session-1","stop_reason":"end_turn"}\n\n'),
		);

		await client.streamChat(
			{ message: "hello" },
			createCallbacks({
				onFinish: (sessionId) => finishes.push(sessionId),
				onError: (error) => errors.push(error),
			}),
		);

		expect(finishes).toEqual(["session-1"]);
		expect(errors).toEqual([]);
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
			'data: {"type":"text_delta","delta":"working 🦀"}\r\n\r\n'
				+ 'data: {"type":"finish","session_id":"session-utf8","stop_reason":"end_turn"}\r\n\r\n',
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
				': heartbeat\r\nevent: message\r\ndata: {"type":"text_delta",\r\ndata: "delta":"joined"}\r\n\r\n'
					+ 'data: {"type":"finish","session_id":"session-2","stop_reason":"end_turn"}\r\n\r\n',
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
		const client = new KrustyClient({
			baseUrl: "https://krusty.test",
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
