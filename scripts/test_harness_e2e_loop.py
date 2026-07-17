#!/usr/bin/env python3
"""Focused dependency-free tests for harness live-steering invariants."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


SCRIPT_PATH = Path(__file__).with_name("harness-e2e-loop.py")
SPEC = importlib.util.spec_from_file_location("harness_e2e_loop", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import bootstrap guard
    raise RuntimeError(f"unable to load {SCRIPT_PATH}")
HARNESS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = HARNESS
SPEC.loader.exec_module(HARNESS)


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
