#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import unittest


def load_module():
    path = Path(__file__).with_name("context-saturation-e2e.py")
    spec = importlib.util.spec_from_file_location("context_saturation_e2e", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


MODULE = load_module()


class ContextSaturationUnitTests(unittest.TestCase):
    def test_corpus_batch_is_deterministic_and_large_enough(self):
        first = MODULE.corpus_batch(1, 10_000)
        second = MODULE.corpus_batch(1, 10_000)
        self.assertEqual(first, second)
        self.assertGreaterEqual(first["characters"], 10_000)
        self.assertGreater(first["records"], 1)
        self.assertIn(MODULE.ORIGIN_AXIOM, first["text"])

    def test_corpus_batches_are_distinct(self):
        first = MODULE.corpus_batch(1, 10_000)
        second = MODULE.corpus_batch(2, 10_000)
        self.assertNotEqual(first["sha256"], second["sha256"])
        self.assertNotEqual(first["first_record_id"], second["first_record_id"])

    def test_compaction_filters(self):
        trace = {
            "events": [
                {"event_type": "context_compaction_started"},
                {"event_type": "context_compacted", "payload": {"compaction_count": 1}},
            ]
        }
        events = [{"type": "text_delta"}, {"type": "context_compacted"}]
        self.assertEqual(len(MODULE.trace_compactions(trace)), 1)
        self.assertEqual(len(MODULE.sse_compactions(events)), 1)

    def test_usage_summary_accumulates_usage_only(self):
        events = [
            {"type": "text_delta", "total_tokens": 99},
            {
                "type": "usage",
                "prompt_tokens": 10,
                "input_tokens": 12,
                "completion_tokens": 3,
                "reasoning_tokens": 2,
                "total_tokens": 15,
            },
            {
                "type": "usage",
                "prompt_tokens": 20,
                "input_tokens": 21,
                "completion_tokens": 4,
                "reasoning_tokens": 1,
                "total_tokens": 25,
            },
        ]
        self.assertEqual(
            MODULE.usage_summary(events),
            {
                "prompt_tokens": 30,
                "input_tokens": 33,
                "completion_tokens": 7,
                "reasoning_tokens": 3,
                "total_tokens": 40,
            },
        )

    def test_require_terra_high_accepts_exact_high_request(self):
        terra = {"key": {"provider": "openai", "model_id": "gpt-5.6-terra"}}
        trace = {
            "events": [
                {
                    "event_type": "provider_request_prepared",
                    "payload": {
                        "diagnostics": {
                            "model_key": terra["key"],
                            "effective_request": {
                                "reasoning_effort": "High",
                                "thinking_enabled": True,
                            },
                        }
                    },
                }
            ]
        }
        effective = MODULE.require_terra_high(trace, terra)
        self.assertEqual(effective["reasoning_effort"], "High")


if __name__ == "__main__":
    unittest.main()
