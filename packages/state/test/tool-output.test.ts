import {
	createDelegatedArtifactState,
	formatToolOutputForDisplay,
	resolveDelegatedKind,
} from "../src/session/delegated.ts";

declare const Deno: {
	test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals<T>(actual: T, expected: T, message: string) {
	if (!Object.is(actual, expected)) {
		throw new Error(
			`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
		);
	}
}

Deno.test("structured Bash policy failures render a concise error", () => {
	const output = JSON.stringify({
		error_code: "blocked_by_policy",
		is_error: true,
		result: {
			error: "Use the dedicated read tool instead.",
			exit_code: null,
			output_preview: "",
		},
		retention: "drop_after_compaction",
		summary: "bash failed (exit 1)",
		tool: "bash",
	});

	assertEquals(
		formatToolOutputForDisplay("bash", output),
		"Use the dedicated read tool instead.",
		"model-facing history metadata must not leak into the mobile tool card",
	);
});

Deno.test("plain command output remains unchanged", () => {
	assertEquals(
		formatToolOutputForDisplay("bash", "tests passed\n"),
		"tests passed\n",
		"plain terminal output should remain byte-for-byte visible",
	);
});

Deno.test("new Agent contract uses capabilities and parent name", () => {
	const args = {
		name: "focused validator",
		instructions: "Run focused checks",
		capabilities: ["execute"],
	};
	assertEquals(
		resolveDelegatedKind("agent", args),
		"explore",
		"non-writing child should use the single-child delegated state family",
	);
	const artifact = createDelegatedArtifactState("explore", args);
	assertEquals(artifact.name, "focused validator", "name must survive presentation seeding");
	assertEquals(artifact.agents[0]?.name, "focused validator", "seed row must use parent name");
	assertEquals(artifact.capabilities?.join(","), "execute", "execute-only must stay exact");
});

Deno.test("legacy agent_type remains a delegated-kind fallback", () => {
	assertEquals(
		resolveDelegatedKind("agent", { agent_type: "verify" }),
		"verify",
		"legacy verifier calls must still replay",
	);
});
