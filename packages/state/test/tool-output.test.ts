import {
	applyDelegatedProgress,
	applyDelegatedSessionState,
	createDelegatedArtifactState,
	formatToolOutputForDisplay,
	parseDelegatedArtifactState,
	resolveDelegatedKind,
} from "../src/session/delegated.ts";
import type {
	DelegatedProgressEvent,
	DelegatedRunResponse,
	DelegatedRunSummaryResponse,
} from "@krusty/api";
import type { ChatMessage, ToolCall } from "../src/session/types.ts";

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

Deno.test("exact capabilities outrank conflicting legacy Agent labels", () => {
	assertEquals(
		resolveDelegatedKind("agent", {
			profile: "verify",
			capabilities: ["read", "write"],
		}),
		"build",
		"write capability must render as a write-capable child",
	);
	assertEquals(
		resolveDelegatedKind("agent", {
			profile: "build",
			capabilities: ["execute"],
		}),
		"explore",
		"legacy build label must not turn execute-only into a writer",
	);
});

Deno.test("empty persisted legacy capabilities defer to the durable run kind", () => {
	assertEquals(
		resolveDelegatedKind(
			"agent",
			{ agent_type: "explore", capabilities: ["read"] },
			"build",
			[],
		),
		"build",
		"migration-era empty capabilities must preserve the persisted legacy role",
	);
});

Deno.test("background Agent result retains a live delegated stage", () => {
	const artifact = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			ok: true,
			data: {
				status: "background_started",
				delegated_run_id: "run-live",
			},
		}),
		{
			name: "runtime audit",
			instructions: "Map runtime boundaries",
			capabilities: ["read"],
		},
	);
	assertEquals(artifact?.stage, "running", "background launch must remain visibly live");
});

Deno.test("single write-capable Agent parses the agents payload", () => {
	const artifact = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			ok: true,
			data: {
				outcome: "success",
				agent_count: 1,
				agents: [{
					agent: "focused writer",
					success: true,
					usable_evidence: true,
					degraded_success: false,
					termination: "completed",
					summary: "Implemented and verified the focused change.",
				}],
			},
		}),
		{
			name: "focused writer",
			capabilities: ["read", "write"],
		},
	);

	assertEquals(artifact?.kind, "build", "write capability must retain build presentation");
	assertEquals(artifact?.agents.length, 1, "single writers must not lose their agents row");
	assertEquals(artifact?.agents[0]?.status, "complete", "usable completed evidence is complete");
	assertEquals(artifact?.agents[0]?.usableEvidence, true, "usable evidence must survive parsing");
});

Deno.test("provider-interrupted retained evidence renders degraded before legacy failure flags", () => {
	const artifact = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			outcome: "partial",
			agents: [{
				agent: "bounded audit",
				success: false,
				usable_evidence: true,
				degraded_success: true,
				termination: "provider_max_tokens",
				error: "provider token limit reached",
				summary: "Retained three source-backed findings.",
			}],
		}),
		{ capabilities: ["read"] },
	);

	assertEquals(artifact?.stage, "degraded", "partial durable outcome must remain degraded");
	assertEquals(artifact?.agents[0]?.status, "degraded", "retained evidence outranks success=false");
	assertEquals(artifact?.agents[0]?.termination, "provider_max_tokens", "termination must survive");
	assertEquals(artifact?.degradedAgents, 1, "degraded rows must contribute to the count");

	const noEvidence = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			outcome: "failed",
			agents: [{
				agent: "empty child",
				success: true,
				usable_evidence: false,
				degraded_success: false,
				termination: "completed",
			}],
		}),
		{ capabilities: ["read"] },
	);
	assertEquals(
		noEvidence?.agents[0]?.status,
		"failed",
		"completed prose without usable evidence must not render complete or degraded",
	);
});

function progressEvent(stage: DelegatedProgressEvent["stage"]): DelegatedProgressEvent {
	return {
		delegated_run_id: `run-${stage}`,
		tool_call_id: "tool-background",
		kind: "explore",
		stage,
		parent_session_id: "session-1",
		task_id: "main",
		agent_name: "runtime audit",
		status: stage === "degraded"
			? "degraded"
			: stage === "cancelled"
				? "cancelled"
				: "failed",
		tool_count: 2,
		tokens: 10,
		current_action: stage,
		completion_summary: null,
		lines_added: 0,
		lines_removed: 0,
		completed_plan_task: null,
	};
}

Deno.test("terminal progress preserves degraded and cancelled outer status", () => {
	const toolCall: ToolCall = {
		id: "tool-background",
		name: "agent",
		arguments: { name: "runtime audit", capabilities: ["read"] },
		status: "success",
		delegated: createDelegatedArtifactState("explore", {
			name: "runtime audit",
			capabilities: ["read"],
		}),
	};

	const degraded = applyDelegatedProgress(toolCall, progressEvent("degraded"));
	assertEquals(degraded.status, "partial", "degraded child work needs a warning status");
	assertEquals(degraded.delegated?.outcome, "partial", "degraded must not become failed");
	assertEquals(degraded.delegated?.agents[0]?.status, "degraded", "live row must be degraded");
	assertEquals(degraded.delegated?.usableAgents, 1, "degraded evidence remains usable");

	const cancelled = applyDelegatedProgress(toolCall, progressEvent("cancelled"));
	assertEquals(cancelled.status, "error", "cancelled work must not keep a success icon");
	assertEquals(cancelled.delegated?.outcome, "cancelled", "cancelled must survive explicitly");
	assertEquals(cancelled.delegated?.agents[0]?.status, "cancelled", "live row must be cancelled");
	assertEquals(cancelled.delegated?.cancelledAgents, 1, "cancelled rows need their own count");
});

Deno.test("durable recent artifacts restore terminal background Agent state", () => {
	const launch = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			ok: true,
			data: { status: "background_started", delegated_run_id: "run-reload" },
		}),
		{ name: "parallel repair", capabilities: ["read", "write"] },
	);
	const messages: ChatMessage[] = [{
		id: "message-1",
		role: "assistant",
		content: "",
		toolCalls: [{
			id: "tool-reload",
			name: "agent",
			arguments: { name: "parallel repair", capabilities: ["read", "write"] },
			status: "success",
			delegated: launch,
		}],
	}];
	const recentRun: DelegatedRunResponse = {
		delegated_run_id: "run-reload",
		parent_tool_call_id: "tool-reload",
		kind: "build",
		stage: "degraded",
		resumable: true,
		child_name: "parallel repair",
		capabilities: ["read", "write"],
		target_scope: [],
		human_review: "One builder completed and one failed.",
		artifact: {
			outcome: "partial",
			agent_count: 2,
			usable_agents: 1,
			failed_agents: 1,
			builders: [
				{
					agent: "repair-a",
					success: true,
					usable_evidence: true,
					degraded_success: false,
					termination: "completed",
					summary: "Completed repair A.",
				},
				{
					agent: "repair-b",
					success: false,
					usable_evidence: false,
					degraded_success: false,
					termination: "failed",
					error: "Focused check failed",
				},
			],
		},
		updated_at: "2026-08-01T12:00:00Z",
	};

	const restored = applyDelegatedSessionState(messages, [], [recentRun]);
	const restoredCall = restored[0]?.toolCalls?.[0];
	assertEquals(restoredCall?.status, "partial", "durable degraded stage must update the card");
	assertEquals(restoredCall?.delegated?.stage, "degraded", "durable stage must survive reload");
	assertEquals(restoredCall?.delegated?.outcome, "partial", "durable outcome must survive reload");
	assertEquals(
		restoredCall?.delegated?.humanReview,
		"One builder completed and one failed.",
		"durable human review must hydrate the card",
	);
	assertEquals(restoredCall?.delegated?.agents.length, 2, "durable builder rows must hydrate");
});

Deno.test("durable terminal state outranks a stale live Running snapshot", () => {
	const toolCall: ToolCall = {
		id: "tool-canonical",
		name: "agent",
		arguments: { name: "canonical child", capabilities: ["read"] },
		status: "running",
		delegatedRunId: "run-canonical",
		delegated: {
			...createDelegatedArtifactState("explore", { name: "canonical child" }),
			delegatedRunId: "run-canonical",
			stage: "running",
		},
	};
	const messages: ChatMessage[] = [{
		id: "message-canonical",
		role: "assistant",
		content: "",
		toolCalls: [toolCall],
	}];
	const recentRun: DelegatedRunResponse = {
		delegated_run_id: "run-canonical",
		parent_tool_call_id: "tool-canonical",
		kind: "explore",
		stage: "complete",
		resumable: true,
		target_scope: [],
		artifact: { outcome: "success", agents: [] },
		updated_at: "2026-08-01T12:00:00Z",
	};
	const staleLive = [{
		delegated_run_id: "run-canonical",
		tool_call_id: "tool-canonical",
		kind: "explore" as const,
		stage: "running" as const,
		parent_session_id: "session-1",
		agents: [],
	}];

	const restored = applyDelegatedSessionState(messages, staleLive, [recentRun]);
	const restoredCall = restored[0]?.toolCalls?.[0];
	assertEquals(restoredCall?.delegated?.stage, "complete", "durable stage must win");
	assertEquals(restoredCall?.status, "success", "durable stage must settle outer status");
});

Deno.test("compact durable summaries settle Agent cards outside the artifact window", () => {
	const launch = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			ok: true,
			data: { status: "background_started", delegated_run_id: "run-old" },
		}),
		{ name: "old child", capabilities: ["read"] },
	);
	const messages: ChatMessage[] = [{
		id: "message-old",
		role: "assistant",
		content: "",
		toolCalls: [{
			id: "tool-old",
			name: "agent",
			arguments: { name: "old child", capabilities: ["read"] },
			status: "running",
			delegatedRunId: "run-old",
			delegated: launch,
		}],
	}];
	const summary: DelegatedRunSummaryResponse = {
		delegated_run_id: "run-old",
		parent_tool_call_id: "tool-old",
		kind: "explore",
		stage: "failed",
		child_name: "old child",
		capabilities: ["read"],
		updated_at: "2026-07-01T12:00:00Z",
	};

	const restored = applyDelegatedSessionState(messages, [], [], [summary]);
	const restoredCall = restored[0]?.toolCalls?.[0];
	assertEquals(restoredCall?.delegated?.stage, "failed", "summary must settle old card");
	assertEquals(restoredCall?.delegated?.outcome, "failed", "summary must settle outcome");
	assertEquals(restoredCall?.status, "error", "summary must settle outer status");
});

Deno.test("late nonterminal progress cannot reopen a terminal delegated run", () => {
	const terminal: ToolCall = {
		id: "tool-terminal",
		name: "agent",
		arguments: { capabilities: ["read"] },
		status: "success",
		delegatedRunId: "run-terminal",
		delegated: {
			...createDelegatedArtifactState("explore", { capabilities: ["read"] }),
			delegatedRunId: "run-terminal",
			stage: "complete",
			outcome: "success",
		},
	};
	const late = progressEvent("running");
	late.delegated_run_id = "run-terminal";
	late.tool_call_id = "tool-terminal";
	late.status = "running";

	assertEquals(
		applyDelegatedProgress(terminal, late),
		terminal,
		"terminal card should be returned unchanged",
	);
});
