import {
	applyDelegatedProgress,
	applyDelegatedSessionState,
	createDelegatedArtifactState,
	formatToolOutputForDisplay,
	mergeDelegatedArtifactState,
	parseDelegatedArtifactState,
	resolveDelegatedKind,
} from "../src/session/delegated.ts";
import type {
	DelegatedProgressEvent,
	DelegatedRunResponse,
	DelegatedRunSummaryResponse,
	DelegationGroupStateResponse,
} from "@mitsuro/api";
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

Deno.test("declared Agent tasks seed a distinct team before durable progress arrives", () => {
	const artifact = createDelegatedArtifactState("build", {
		name: "release-team",
		tasks: [
			{ id: "api", name: "API builder", instructions: "Implement the API" },
			{ id: "ui", name: "UI builder", scope: "Build the interface" },
		],
	});

	assertEquals(artifact.agents.length, 2, "every declared task should seed one table row");
	assertEquals(artifact.agents[0]?.taskId, "declared:api", "declared ids should stay stable");
	assertEquals(artifact.agents[0]?.name, "API builder", "task name should drive the row");
	assertEquals(
		artifact.agents[1]?.currentAction,
		"Build the interface",
		"scope should provide useful pre-admission activity copy",
	);
});

Deno.test("canonical progress replaces optimistic rows without growing the team", () => {
	const args = {
		capabilities: ["read"],
		tasks: [
			{ id: "first", name: "First child" },
			{ id: "second", name: "Second child" },
		],
	};
	let call: ToolCall = {
		id: "tool-team",
		name: "agent",
		arguments: args,
		status: "running",
		delegated: createDelegatedArtifactState("explore", args),
	};
	for (const ordinal of [1, 0]) {
		call = applyDelegatedProgress(call, {
			parent_session_id: "session-1",
			tool_call_id: "tool-team",
			delegated_run_id: "group-team",
			task_id: `group-team:task:${ordinal}`,
			agent_name: ordinal === 0 ? "First child" : "Second child",
			kind: "explore",
			stage: "running",
			status: "running",
			tool_count: 1,
			tokens: 10,
			current_action: "Reading",
			completion_summary: null,
			lines_added: 0,
			lines_removed: 0,
			completed_plan_task: null,
		});
		assertEquals(call.delegated?.agents.length, 2, "admission must not add a third row");
	}
	assertEquals(
		call.delegated?.agents.map((agent) => agent.taskId).join(","),
		"group-team:task:0,group-team:task:1",
		"canonical rows must retain declared task order",
	);
});

Deno.test("terminal report labels do not duplicate canonical live rows", () => {
	const current = {
		...createDelegatedArtifactState("explore", { capabilities: ["read"] }),
		delegatedRunId: "group-team",
		stage: "running" as const,
		agents: [
			{ taskId: "group-team:task:0", name: "First child", status: "running" as const, toolCount: 1, tokens: 10, linesAdded: 0, linesRemoved: 0 },
			{ taskId: "group-team:task:1", name: "Second child", status: "running" as const, toolCount: 1, tokens: 10, linesAdded: 0, linesRemoved: 0 },
		],
	};
	const terminal = parseDelegatedArtifactState(
		"agent",
		JSON.stringify({
			outcome: "success",
			delegated_run_id: "group-team",
			agents: [
				{ agent: "Hive Worker 01", success: true, usable_evidence: true },
				{ agent: "Hive Worker 02", success: true, usable_evidence: true },
			],
		}),
		{ capabilities: ["read"] },
	);
	if (!terminal) throw new Error("expected terminal artifact");
	const merged = mergeDelegatedArtifactState(current, terminal);
	assertEquals(merged.agents.length, 2, "terminal reports must not append duplicate rows");
	assertEquals(merged.agents[0]?.taskId, "group-team:task:0", "canonical identity must survive");
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

Deno.test("canonical delegation groups restore parallel task state after reconnect", () => {
	const messages: ChatMessage[] = [{
		id: "message-group",
		role: "assistant",
		content: "",
		toolCalls: [{
			id: "tool-group",
			name: "agent",
			arguments: { capabilities: ["write"], components: ["api", "ui"] },
			status: "running",
		}],
	}];
	const group: DelegationGroupStateResponse = {
		delegation_group_id: "group-1",
		parent_tool_call_id: "tool-group",
		state: "running",
		execution_mode: "detached",
		parent_continuation_state: "pending",
		updated_at: "2026-08-08T12:00:00Z",
		tasks: [
			{
				delegation_task_id: "task-api",
				task_key: "api",
				role: "build",
				state: "running",
				attempt_count: 1,
				updated_at: "2026-08-08T12:00:00Z",
			},
			{
				delegation_task_id: "task-ui",
				task_key: "ui",
				role: "build",
				state: "leased",
				attempt_count: 0,
				updated_at: "2026-08-08T12:00:00Z",
			},
			{
				delegation_task_id: "task-verify",
				task_key: "verify",
				role: "verifier",
				state: "queued",
				attempt_count: 0,
				depends_on: ["api", "ui"],
				updated_at: "2026-08-08T12:00:00Z",
			},
		],
	};

	const live = applyDelegatedProgress(messages[0]!.toolCalls![0]!, {
		parent_session_id: "session-1",
		tool_call_id: "tool-group",
		delegated_run_id: "group-1",
		task_id: "task-api",
		agent_name: "API Builder",
		kind: "build",
		stage: "running",
		status: "running",
		tool_count: 7,
		tokens: 321,
		current_action: "Running API tests",
		completion_summary: null,
		lines_added: 12,
		lines_removed: 3,
		completed_plan_task: null,
	});
	messages[0]!.toolCalls = [live];
	const restored = applyDelegatedSessionState(messages, [], [], [], [group]);
	const delegated = restored[0]?.toolCalls?.[0]?.delegated;
	assertEquals(delegated?.delegatedRunId, "group-1", "group identity must win");
	assertEquals(delegated?.groupState, "running", "exact group state must be retained");
	assertEquals(delegated?.agents.length, 3, "all logical tasks must render");
	assertEquals(delegated?.agents[0]?.status, "running", "active task must remain running");
	assertEquals(delegated?.agents[0]?.name, "API Builder", "live display name must survive snapshot");
	assertEquals(delegated?.agents[0]?.toolCount, 7, "live metrics must survive snapshot");
	assertEquals(delegated?.agents[0]?.attemptCount, 1, "durable attempts must win");
	assertEquals(
		delegated?.agents[0]?.currentAction,
		"Running API tests",
		"live action must survive a running durable snapshot",
	);
	assertEquals(delegated?.agents[1]?.status, "pending", "leased task remains non-running");
	assertEquals(delegated?.agents[1]?.taskState, "leased", "exact leased state must be retained");
	assertEquals(
		delegated?.agents[1]?.currentAction,
		"Waiting for provider capacity",
		"capacity wait must be distinct from running",
	);
	assertEquals(
		delegated?.agents[2]?.currentAction,
		"Waiting for api, ui",
		"dependency wait must be visible",
	);
	assertEquals(delegated?.activeTargets, 1, "active count must come from group tasks");
	assertEquals(delegated?.waitingTargets, 1, "leased tasks must not inflate running count");
	assertEquals(delegated?.pendingTargets, 1, "pending count must come from group tasks");
});

Deno.test("canonical cancelled group replaces stale running UI state after reconnect", () => {
	const messages: ChatMessage[] = [{
		id: "message-cancelled-group",
		role: "assistant",
		content: "",
		toolCalls: [{
			id: "tool-cancelled-group",
			name: "agent",
			arguments: { capabilities: ["write", "execute"] },
			status: "running",
		}],
	}];
	const live = applyDelegatedProgress(messages[0]!.toolCalls![0]!, {
		...progressEvent("running"),
		tool_call_id: "tool-cancelled-group",
		delegated_run_id: "group-cancelled",
		task_id: "task-builder",
		agent_name: "Builder",
	});
	messages[0]!.toolCalls = [live];
	const group: DelegationGroupStateResponse = {
		delegation_group_id: "group-cancelled",
		parent_tool_call_id: "tool-cancelled-group",
		state: "cancelled",
		execution_mode: "foreground",
		parent_continuation_state: "not_requested",
		updated_at: "2026-08-11T12:00:00Z",
		tasks: [{
			delegation_task_id: "task-builder",
			task_key: "builder",
			role: "build",
			state: "cancelled",
			attempt_count: 1,
			updated_at: "2026-08-11T12:00:00Z",
		}],
	};

	const restored = applyDelegatedSessionState(messages, [], [], [], [group]);
	const call = restored[0]?.toolCalls?.[0];

	assertEquals(call?.delegated?.groupState, "cancelled", "durable group state must win");
	assertEquals(call?.delegated?.outcome, "cancelled", "cancelled outcome must survive");
	assertEquals(
		call?.delegated?.agents[0]?.status,
		"cancelled",
		"durable task state must replace stale running progress",
	);
	assertEquals(call?.delegated?.cancelledAgents, 1, "cancelled task count must be canonical");
	assertEquals(call?.status, "error", "cancelled work must not retain a running card");
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
