import { formatToolOutputForDisplay } from "../src/session/delegated.ts";

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
