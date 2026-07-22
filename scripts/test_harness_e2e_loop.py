#!/usr/bin/env python3
"""Focused dependency-free tests for harness live-steering invariants."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest
from typing import Any


SCRIPT_PATH = Path(__file__).with_name("harness-e2e-loop.py")
SPEC = importlib.util.spec_from_file_location("harness_e2e_loop", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import bootstrap guard
    raise RuntimeError(f"unable to load {SCRIPT_PATH}")
HARNESS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = HARNESS
SPEC.loader.exec_module(HARNESS)


class SequenceTraceApi:
    def __init__(self, responses: list[dict[str, Any]]) -> None:
        self.responses = responses
        self.calls = 0

    def json_request(self, method: str, path: str) -> dict[str, Any]:
        if method != "GET" or "/trace?limit=1000" not in path:
            raise AssertionError(f"unexpected request: {method} {path}")
        index = min(self.calls, len(self.responses) - 1)
        self.calls += 1
        return self.responses[index]


def trace_response(events: list[dict[str, Any]]) -> dict[str, Any]:
    stop_reason = next(
        (
            event.get("stop_reason")
            for event in reversed(events)
            if event.get("event_type") == "finished"
        ),
        None,
    )
    return {
        "events": events,
        "latest_sequence": events[-1]["sequence"] if events else None,
        "summary": {"last_stop_reason": stop_reason},
    }


def trace_event(
    sequence: int,
    event_type: str,
    *,
    run_id: str = "run-1",
    stop_reason: str | None = None,
) -> dict[str, Any]:
    event: dict[str, Any] = {
        "sequence": sequence,
        "run_id": run_id,
        "event_type": event_type,
        "payload": {},
    }
    if stop_reason is not None:
        event["stop_reason"] = stop_reason
        event["payload"] = {"stop_reason": stop_reason}
    return event


def budget_event(
    sequence: int,
    run_id: str,
    *,
    max_turns: int | None = None,
    source: str = "unlimited_default",
) -> dict[str, Any]:
    event = trace_event(sequence, "run_budget_resolved", run_id=run_id)
    event["payload"] = {"max_turns": max_turns, "source": source}
    return event


class CandidateSafetyTests(unittest.TestCase):
    def test_candidate_url_requires_loopback_nonproduction_port(self) -> None:
        self.assertEqual(
            HARNESS.validate_candidate_base_url("http://127.0.0.1:3100"),
            "http://127.0.0.1:3100",
        )
        with self.assertRaises(HARNESS.AcceptanceFailure):
            HARNESS.validate_candidate_base_url("http://127.0.0.1:3000")
        with self.assertRaises(HARNESS.AcceptanceFailure):
            HARNESS.validate_candidate_base_url("http://honey:3100")


class TraceConvergenceTests(unittest.TestCase):
    def test_waits_through_async_batches_until_terminal_call_is_accounted(self) -> None:
        thinking = trace_event(1, "thinking_delta")
        finished = trace_event(2, "finished", stop_reason="completed")
        provider_call = trace_event(3, "provider_call")
        api = SequenceTraceApi(
            [
                trace_response([thinking]),
                trace_response([thinking, finished]),
                trace_response([thinking, finished, provider_call]),
            ]
        )

        trace, summary = HARNESS.wait_for_completed_trace_run(
            api,
            "session-1",
            "trace convergence",
            timeout=1.0,
            poll_interval=0.0,
        )

        self.assertEqual(api.calls, 3)
        self.assertEqual(trace["latest_sequence"], 3)
        self.assertEqual(summary["last_stop_reason"], "completed")

    def test_ignores_prior_terminal_and_requires_a_run_after_cursor(self) -> None:
        prior = [
            trace_event(1, "finished", run_id="run-1", stop_reason="completed"),
            trace_event(2, "provider_call", run_id="run-1"),
        ]
        next_run = [
            trace_event(3, "finished", run_id="run-2", stop_reason="completed"),
            trace_event(4, "provider_call", run_id="run-2"),
        ]
        api = SequenceTraceApi(
            [trace_response(prior), trace_response(prior + next_run)]
        )

        trace, _ = HARNESS.wait_for_completed_trace_run(
            api,
            "session-1",
            "trace cursor",
            after_sequence=2,
            timeout=1.0,
            poll_interval=0.0,
        )

        self.assertEqual(api.calls, 2)
        self.assertEqual(trace["latest_sequence"], 4)

    def test_cursor_waits_through_budget_finished_and_accounting_batches(self) -> None:
        prior = [
            budget_event(1, "run-1"),
            trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
            trace_event(3, "provider_call", run_id="run-1"),
        ]
        next_budget = budget_event(4, "run-2")
        next_finished = trace_event(
            5, "finished", run_id="run-2", stop_reason="completed"
        )
        next_provider_call = trace_event(6, "provider_call", run_id="run-2")
        api = SequenceTraceApi(
            [
                trace_response(prior),
                trace_response(prior + [next_budget]),
                trace_response(prior + [next_budget, next_finished]),
                trace_response(
                    prior + [next_budget, next_finished, next_provider_call]
                ),
            ]
        )

        trace, _ = HARNESS.wait_for_completed_trace_run(
            api,
            "session-1",
            "batched trace cursor",
            after_sequence=3,
            timeout=1.0,
            poll_interval=0.0,
        )

        self.assertEqual(api.calls, 4)
        self.assertEqual(trace["latest_sequence"], 6)

    def test_incomplete_run_timeout_reports_boundary_run_ids(self) -> None:
        incomplete = [
            budget_event(1, "run-1"),
            trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
        ]
        api = SequenceTraceApi([trace_response(incomplete)])

        with self.assertRaisesRegex(
            HARNESS.AcceptanceFailure,
            "finished_run_ids=\\['run-1'\\].*provider_call_run_ids=\\[\\]",
        ):
            HARNESS.wait_for_completed_trace_run(
                api,
                "session-1",
                "incomplete trace",
                timeout=0.0,
                poll_interval=0.0,
            )

        self.assertEqual(api.calls, 1)

    def test_fails_immediately_on_new_non_completed_terminal(self) -> None:
        failed = trace_event(1, "finished", stop_reason="provider_error")
        api = SequenceTraceApi([trace_response([failed])])

        with self.assertRaisesRegex(
            HARNESS.AcceptanceFailure, "newly persisted run did not complete"
        ):
            HARNESS.wait_for_completed_trace_run(
                api,
                "session-1",
                "trace failure",
                timeout=1.0,
                poll_interval=0.0,
            )

        self.assertEqual(api.calls, 1)

    def test_settled_trace_waits_for_expected_followup_and_accounting(self) -> None:
        prior = [
            budget_event(1, "run-1"),
            trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
            trace_event(3, "provider_call", run_id="run-1"),
        ]
        next_budget = budget_event(4, "run-2")
        next_finished = trace_event(
            5, "finished", run_id="run-2", stop_reason="completed"
        )
        next_provider_call = trace_event(6, "provider_call", run_id="run-2")
        responses = [
            trace_response(prior),
            trace_response(prior + [next_budget]),
            trace_response(prior + [next_budget, next_finished]),
            trace_response(prior + [next_budget, next_finished, next_provider_call]),
        ]
        for response in responses:
            response["summary"]["total_runs"] = len(
                {
                    event["run_id"]
                    for event in response["events"]
                    if event.get("event_type") == "run_budget_resolved"
                }
            )
        api = SequenceTraceApi(responses)

        trace, summary = HARNESS.wait_for_settled_trace_runs(
            api,
            "session-1",
            "settled trace",
            expected_runs=2,
            timeout=1.0,
            poll_interval=0.0,
        )

        self.assertEqual(api.calls, 4)
        self.assertEqual(trace["latest_sequence"], 6)
        self.assertEqual(summary["total_runs"], 2)

    def test_settled_trace_rejects_terminal_without_budget_immediately(self) -> None:
        terminal = trace_event(1, "finished", stop_reason="completed")
        response = trace_response([terminal])
        response["summary"]["total_runs"] = 1
        api = SequenceTraceApi([response])

        with self.assertRaisesRegex(
            HARNESS.AcceptanceFailure, "terminal runs lacked prior budget events"
        ):
            HARNESS.wait_for_settled_trace_runs(
                api,
                "session-1",
                "contradictory trace",
                expected_runs=1,
                timeout=1.0,
                poll_interval=0.0,
            )

        self.assertEqual(api.calls, 1)

    def test_settled_trace_timeout_reports_all_run_boundaries(self) -> None:
        incomplete = [
            budget_event(1, "run-1"),
            trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
        ]
        response = trace_response(incomplete)
        response["summary"]["total_runs"] = 1
        api = SequenceTraceApi([response])

        with self.assertRaisesRegex(
            HARNESS.AcceptanceFailure,
            "budget_run_ids=\\['run-1'\\].*finished_run_ids=\\['run-1'\\].*"
            "provider_call_run_ids=\\[\\]",
        ):
            HARNESS.wait_for_settled_trace_runs(
                api,
                "session-1",
                "incomplete settled trace",
                expected_runs=1,
                timeout=0.0,
                poll_interval=0.0,
            )

        self.assertEqual(api.calls, 1)


class RunBudgetTraceTests(unittest.TestCase):
    def test_accepts_one_unlimited_default_budget_per_completed_run(self) -> None:
        trace = trace_response(
            [
                budget_event(1, "run-1"),
                trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
                budget_event(3, "run-2"),
                trace_event(4, "finished", run_id="run-2", stop_reason="completed"),
                budget_event(5, "run-3"),
                trace_event(6, "finished", run_id="run-3", stop_reason="completed"),
            ]
        )

        budgets = HARNESS.validate_trace_run_budgets(trace, "multi-run trace")

        self.assertEqual(
            [budget["run_id"] for budget in budgets],
            ["run-1", "run-2", "run-3"],
        )
        self.assertTrue(all(budget["max_turns"] is None for budget in budgets))
        self.assertTrue(
            all(budget["source"] == "unlimited_default" for budget in budgets)
        )

    def test_rejects_duplicate_or_missing_budget_events_by_run_id(self) -> None:
        duplicate = trace_response(
            [
                budget_event(1, "run-1"),
                budget_event(2, "run-1"),
                trace_event(3, "finished", run_id="run-1", stop_reason="completed"),
            ]
        )
        with self.assertRaisesRegex(HARNESS.AcceptanceFailure, "emitted 2 budget events"):
            HARNESS.validate_trace_run_budgets(duplicate, "duplicate trace")

        missing = trace_response(
            [trace_event(1, "finished", run_id="run-1", stop_reason="completed")]
        )
        with self.assertRaisesRegex(HARNESS.AcceptanceFailure, "run ids did not match"):
            HARNESS.validate_trace_run_budgets(missing, "missing trace")

    def test_rejects_a_hidden_or_non_default_cap(self) -> None:
        capped = trace_response(
            [
                budget_event(1, "run-1", max_turns=50, source="explicit_run"),
                trace_event(2, "finished", run_id="run-1", stop_reason="completed"),
            ]
        )
        with self.assertRaisesRegex(HARNESS.AcceptanceFailure, "hidden or non-default"):
            HARNESS.validate_trace_run_budgets(capped, "capped trace")


class LiveSteeringValidationTests(unittest.TestCase):
    pending_id = "pending-steer-1"
    steering_message = "Do not call another tool. Reply exactly STEERED:marker"
    expected_reply = "STEERED:marker"
    command = "python3 live_steering_probe.py"
    ready = "READY:marker"

    def valid_events(self) -> list[dict[str, object]]:
        return [
            {"type": "tool_call_start", "id": "call-1", "name": "bash"},
            {
                "type": "tool_call_complete",
                "id": "call-1",
                "name": "bash",
                "arguments": {
                    "command": self.command,
                    "timeout": 60_000,
                    "run_in_background": False,
                },
            },
            {"type": "tool_executing", "id": "call-1", "name": "bash"},
            {
                "type": "tool_output_delta",
                "id": "call-1",
                "delta": f"{self.ready}\n",
            },
            {
                "type": "tool_result",
                "id": "call-1",
                "output": '{"ok":true}',
                "is_error": False,
            },
            {"type": "turn_complete", "turn": 1, "has_more": True},
            {
                "type": "steering_injected",
                "pending_id": self.pending_id,
                "message": self.steering_message,
            },
            {"type": "text_delta", "delta": self.expected_reply},
            {"type": "turn_complete", "turn": 2, "has_more": False},
            {
                "type": "finish",
                "session_id": "session-1",
                "stop_reason": "completed",
            },
        ]

    def test_accepts_one_mid_tool_steering_lifecycle(self) -> None:
        evidence = HARNESS.validate_live_steering_stream(
            self.valid_events(),
            "test lane",
            pending_id=self.pending_id,
            steering_message=self.steering_message,
            expected_reply=self.expected_reply,
            expected_command=self.command,
            ready_marker=self.ready,
        )

        self.assertEqual(evidence["tool_call_id"], "call-1")
        self.assertLess(
            evidence["ready_index"], evidence["steering_injected_index"]
        )
        self.assertEqual(evidence["reply"], self.expected_reply)

    def test_rejects_duplicate_steering_injection(self) -> None:
        events = self.valid_events()
        events.insert(7, dict(events[6]))

        with self.assertRaises(HARNESS.AcceptanceFailure):
            HARNESS.validate_live_steering_stream(
                events,
                "test lane",
                pending_id=self.pending_id,
                steering_message=self.steering_message,
                expected_reply=self.expected_reply,
                expected_command=self.command,
                ready_marker=self.ready,
            )

    def test_requires_one_canonical_steering_and_no_pending_role(self) -> None:
        messages = [
            {
                "role": "user",
                "content": [{"type": "text", "text": "initial prompt"}],
            },
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": self.steering_message}
                ],
            },
            {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": self.expected_reply}
                ],
            },
        ]
        evidence = HARNESS.validate_live_steering_messages(
            messages,
            "test lane",
            steering_message=self.steering_message,
            expected_reply=self.expected_reply,
        )
        self.assertEqual(evidence["steering_message_index"], 1)
        self.assertEqual(evidence["steered_reply_index"], 2)

        messages.insert(
            2,
            {
                "role": f"pending_user:{self.pending_id}",
                "content": [
                    {"type": "text", "text": self.steering_message}
                ],
            },
        )
        with self.assertRaises(HARNESS.AcceptanceFailure):
            HARNESS.validate_live_steering_messages(
                messages,
                "test lane",
                steering_message=self.steering_message,
                expected_reply=self.expected_reply,
            )


if __name__ == "__main__":
    unittest.main()
