#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("grok-core-behavior.py")
SPEC = importlib.util.spec_from_file_location("grok_core_behavior", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
BEHAVIOR = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = BEHAVIOR
SPEC.loader.exec_module(BEHAVIOR)


class FakeApi:
    def __init__(self, response):
        self.response = response

    def json_request(self, method, path):
        self.assert_request = (method, path)
        return self.response


class GrokCoreBehaviorTests(unittest.TestCase):
    def test_progress_guard_action_reads_nested_sse_telemetry(self):
        event = {
            "type": "progress_guard",
            "telemetry": {"action": "replan", "intent_hash": "abc"},
        }
        self.assertEqual(BEHAVIOR.progress_guard_action(event), "replan")

    def test_tree_snapshot_detects_content_and_path_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "one.txt").write_text("one")
            first = BEHAVIOR.tree_snapshot(root)
            (root / "one.txt").write_text("two")
            (root / "two.txt").write_text("extra")
            second = BEHAVIOR.tree_snapshot(root)
        self.assertNotEqual(first, second)
        self.assertEqual(set(second), {"one.txt", "two.txt"})

    def test_select_exact_model_requires_one_grok_tool_row(self):
        key = {
            "provider": "grok",
            "model_id": "grok-4.5",
            "auth_scope": None,
            "api_format": "open_ai_responses",
        }
        row = {
            "id": "grok-4.5",
            "provider_id": "grok",
            "key": key,
            "catalog_source": "live_dynamic",
            "catalog_revision": "grok-live-1",
            "supports_tools": True,
            "context_window": 500_000,
            "max_output": 32_768,
        }
        api = FakeApi({"models": [row]})
        self.assertEqual(BEHAVIOR.select_exact_model(api, "grok-4.5"), row)
        self.assertEqual(api.assert_request, ("GET", "/api/models"))

    def test_runtime_contract_requires_exact_key_and_unlimited_budget(self):
        key = {
            "provider": "grok",
            "model_id": "grok-4.5",
            "auth_scope": None,
            "api_format": "open_ai_responses",
        }
        diagnostics = {
            "model_key": key,
            "catalog_revision": "catalog-1",
            "effective_request": {"model": "grok-4.5"},
            "prompt_manifest": {"prompt_hash": "a" * 64},
        }
        stream = [
            {"type": "run_budget_resolved", "max_turns": None, "source": "unlimited"},
            {"type": "provider_request_prepared", "diagnostics": diagnostics},
        ]
        trace = {
            "events": [
                {
                    "event_type": "provider_request_prepared",
                    "payload": {"diagnostics": diagnostics},
                }
            ]
        }
        result = BEHAVIOR.validate_runtime_contract(
            stream,
            trace,
            {"key": key, "catalog_revision": "catalog-1"},
            "test",
        )
        self.assertIsNone(result["run_budget"]["max_turns"])
        self.assertEqual(result["stream_request_count"], 1)
        self.assertEqual(result["trace_request_count"], 1)


if __name__ == "__main__":
    unittest.main()
