import { describe, expect, test } from "bun:test";

import { normalizeUsageMetrics } from "../src/client";

describe("normalizeUsageMetrics", () => {
	test("retains uncached, cache, output, and logical totals", () => {
		expect(
			normalizeUsageMetrics({
				type: "usage",
				prompt_tokens: 100,
				input_tokens: 1_000,
				completion_tokens: 50,
				reasoning_tokens: 40,
				cache_creation_input_tokens: 200,
				cache_read_input_tokens: 700,
				total_tokens: 1_050,
			}),
		).toEqual({
			promptTokens: 100,
			inputTokens: 1_000,
			completionTokens: 50,
			reasoningTokens: 40,
			cacheCreationInputTokens: 200,
			cacheReadInputTokens: 700,
			totalTokens: 1_050,
		});
	});

	test("derives logical input and total for older servers", () => {
		expect(
			normalizeUsageMetrics({
				type: "usage",
				prompt_tokens: 100,
				completion_tokens: 50,
				cache_creation_input_tokens: 200,
				cache_read_input_tokens: 700,
			}),
		).toMatchObject({ inputTokens: 1_000, totalTokens: 1_050 });
	});

	test("preserves a provider total larger than represented buckets", () => {
		expect(
			normalizeUsageMetrics({
				type: "usage",
				prompt_tokens: 1_000,
				input_tokens: 1_000,
				completion_tokens: 50,
				total_tokens: 1_550,
			}),
		).toMatchObject({ totalTokens: 1_550 });
	});

	test("does not add the reasoning subset to completion totals", () => {
		expect(
			normalizeUsageMetrics({
				type: "usage",
				prompt_tokens: 1_000,
				completion_tokens: 550,
				reasoning_tokens: 500,
				total_tokens: 1_550,
			}),
		).toMatchObject({
			completionTokens: 550,
			reasoningTokens: 500,
			totalTokens: 1_550,
		});
	});

	test("repairs a zero legacy total from represented buckets", () => {
		expect(
			normalizeUsageMetrics({
				type: "usage",
				prompt_tokens: 100,
				completion_tokens: 50,
				total_tokens: 0,
			}),
		).toMatchObject({ totalTokens: 150 });
	});
});
