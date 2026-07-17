import { describe, expect, it } from "bun:test";

import { KrustyApiError, KrustyClient } from "../src";

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
