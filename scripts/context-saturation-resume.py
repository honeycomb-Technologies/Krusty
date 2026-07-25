#!/usr/bin/env python3
"""Resume a compacted Context Atlas run after a model-corrected tool error."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time
from typing import Any


def load_saturation() -> Any:
    path = Path(__file__).with_name("context-saturation-e2e.py")
    spec = importlib.util.spec_from_file_location("context_saturation_e2e", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load saturation helpers from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


SATURATION = load_saturation()
HARNESS = SATURATION.HARNESS


def first_record_id(project_dir: Path) -> str:
    path = project_dir / "corpus" / "batch-001.ndjson"
    HARNESS.require(path.is_file(), f"first corpus shard was absent: {path}")
    with path.open() as handle:
        first = json.loads(handle.readline())
    value = first.get("record_id")
    HARNESS.require(isinstance(value, str) and bool(value), "first record ID was absent")
    return value


def run(args: argparse.Namespace) -> dict[str, Any]:
    base_url = HARNESS.validate_candidate_base_url(args.base_url)
    project_dir = args.root / "context-atlas"
    original_path = args.root / "context-saturation-summary.json"
    HARNESS.require(original_path.is_file(), f"original summary was absent: {original_path}")
    original = json.loads(original_path.read_text())
    session_id = args.session_id or original.get("grok_session_id")
    HARNESS.require(isinstance(session_id, str) and bool(session_id), "Grok session ID absent")
    api = HARNESS.KrustyApi(base_url, args.timeout)

    grok = HARNESS.select_stable_exact_model(
        api, args.grok_model, provider_id="grok", timeout=60
    )
    terra = HARNESS.select_stable_exact_model(
        api, args.terra_model, provider_id="open_a_i", timeout=60
    )
    trace = api.json_request("GET", f"/api/sessions/{session_id}/trace?limit=1000")
    compactions = SATURATION.trace_compactions(trace)
    HARNESS.require(compactions, "resume session had no durable compaction")
    compaction = compactions[-1]
    payload = compaction.get("payload", {})
    HARNESS.require(payload.get("reason") == "auto", f"compaction was not automatic: {payload}")
    HARNESS.require(
        payload.get("estimated_tokens_before", 0) > payload.get("estimated_tokens_after", 0),
        f"compaction did not reduce context: {payload}",
    )
    trace_summary = trace.get("summary", {})
    HARNESS.require(trace_summary.get("last_stop_reason") == "completed", "failed run did not recover")
    HARNESS.require(trace_summary.get("tool_errors", 0) >= 1, "recovered tool error was not retained")
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, session_id), "compacted Grok session before resume"
    )

    index = SATURATION.load_index(project_dir)
    batches = index.get("batches")
    HARNESS.require(isinstance(batches, list) and len(batches) >= 3, "Batch 3 was not indexed")
    initial_tests = SATURATION.run_project_tests(project_dir)
    cursor = trace.get("latest_sequence", 0)
    result: dict[str, Any] = {
        "status": "running",
        "root": str(args.root),
        "project_dir": str(project_dir),
        "grok_session_id": session_id,
        "grok": SATURATION.compact_model(grok),
        "terra": SATURATION.compact_model(terra),
        "recovered_run": {
            "tool_errors": trace_summary.get("tool_errors"),
            "last_stop_reason": trace_summary.get("last_stop_reason"),
            "compaction": payload,
            "indexed_batches": len(batches),
            "tests": initial_tests,
        },
    }
    output_path = args.root / "context-saturation-continuation-summary.json"
    SATURATION.persist_summary(args.root, original)
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")

    continuity_marker = "POST-COMPACTION-CONTINUITY-OK"
    continuity_events = api.chat(
        SATURATION.chat_payload(
            session_id,
            "Without calling any tool, state the exact origin axiom established at the start "
            f"of this project, followed by `{continuity_marker}`. Do not explain.",
            None,
            thinking="off",
        )
    )
    continuity_text = SATURATION.completed_text(continuity_events, "Grok continuity resume")
    HARNESS.require(not HARNESS.tool_calls(continuity_events), "continuity used a tool")
    HARNESS.require(SATURATION.ORIGIN_AXIOM in continuity_text, "origin axiom was lost")
    HARNESS.require(continuity_marker in continuity_text, "continuity marker absent")
    continuity_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, session_id, "Grok continuity resume trace", after_sequence=cursor, timeout=15
    )
    cursor = continuity_trace["latest_sequence"]
    result["post_compaction_continuity"] = {
        "status": "pass",
        "used_tools": False,
        "usage": SATURATION.usage_summary(continuity_events),
    }
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")

    record_id = first_record_id(project_dir)
    lineage_events = api.chat(
        SATURATION.chat_payload(
            session_id,
            "Continue building Context Atlas after compaction. Add `lineage --index "
            "atlas-index.json --record-id RECORD_ID`, emitting deterministic JSON for the "
            "matching record and dependency and failing for an unknown record. Add tests, "
            "document it, run the full suite, write docs/POST_COMPACTION.md, demonstrate "
            f"record `{record_id}`, and finish with `POST-COMPACTION-BUILD-OK`.",
            None,
            thinking="off",
        )
    )
    lineage_text = SATURATION.completed_text(lineage_events, "Grok resumed build")
    HARNESS.require("POST-COMPACTION-BUILD-OK" in lineage_text, "lineage marker absent")
    lineage_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, session_id, "Grok resumed build trace", after_sequence=cursor, timeout=15
    )
    cursor = lineage_trace["latest_sequence"]
    lineage = subprocess.run(
        [
            sys.executable,
            "context_atlas.py",
            "lineage",
            "--index",
            "atlas-index.json",
            "--record-id",
            record_id,
        ],
        cwd=project_dir,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    HARNESS.require(lineage.returncode == 0, f"lineage failed: {lineage.stderr}")
    lineage_json = json.loads(lineage.stdout)
    HARNESS.require(record_id in json.dumps(lineage_json), "lineage omitted record")
    result["post_compaction_build"] = {
        "status": "pass",
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(lineage_events)],
        "usage": SATURATION.usage_summary(lineage_events),
        "tests": SATURATION.run_project_tests(project_dir),
        "lineage_output": lineage_json,
    }
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, session_id), "Grok session after resumed build"
    )
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")

    terra_session = SATURATION.create_session(
        api, project_dir, terra, "Terra High Context Atlas audit after compaction"
    )
    result["terra_session_id"] = terra_session
    terra_events = api.chat(
        SATURATION.chat_payload(
            terra_session,
            "Audit and extend the existing Context Atlas project without replacing its "
            "architecture. Implement `report --index atlas-index.json --format json`; it "
            "must verify first and emit exactly `total_batches`, `total_records`, "
            f"`origin_axiom`, and `integrity_status`. origin_axiom is `{SATURATION.ORIGIN_AXIOM}` "
            "and integrity_status must be `ok`. Add tests, update README.md, create "
            "docs/TERRA_AUDIT.md with concrete findings, run all tests, demonstrate the "
            "report, and finish with `TERRA-HIGH-AUDIT-OK`.",
            terra,
            thinking="high",
        )
    )
    terra_text = SATURATION.completed_text(terra_events, "Terra High resumed audit")
    HARNESS.require("TERRA-HIGH-AUDIT-OK" in terra_text, "Terra marker absent")
    terra_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, terra_session, "Terra High resumed trace", after_sequence=0, timeout=20
    )
    effective_request = SATURATION.require_terra_high(terra_trace, terra)
    report = subprocess.run(
        [
            sys.executable,
            "context_atlas.py",
            "report",
            "--index",
            "atlas-index.json",
            "--format",
            "json",
        ],
        cwd=project_dir,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    HARNESS.require(report.returncode == 0, f"Terra report failed: {report.stderr}")
    report_json = json.loads(report.stdout)
    expected_keys = {"total_batches", "total_records", "origin_axiom", "integrity_status"}
    HARNESS.require(set(report_json) == expected_keys, f"report schema drifted: {report_json}")
    HARNESS.require(report_json["origin_axiom"] == SATURATION.ORIGIN_AXIOM, "axiom drifted")
    HARNESS.require(report_json["integrity_status"] == "ok", "integrity was not ok")
    HARNESS.require(report_json["total_batches"] == len(batches), "batch total drifted")
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, terra_session), "Terra session after audit"
    )
    result["terra_high"] = {
        "status": "pass",
        "effective_request": effective_request,
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(terra_events)],
        "usage": SATURATION.usage_summary(terra_events),
        "tests": SATURATION.run_project_tests(project_dir),
        "report": report_json,
    }
    result["status"] = "pass"
    result["completed_at_unix"] = int(time.time())
    output_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2, sort_keys=True))
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:3100")
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--session-id")
    parser.add_argument("--grok-model", default="grok-4.5")
    parser.add_argument("--terra-model", default="gpt-5.6-terra")
    parser.add_argument("--timeout", type=float, default=1_200.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    output = args.root / "context-saturation-continuation-summary.json"
    try:
        run(args)
    except Exception as error:
        try:
            result = json.loads(output.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            result = {}
        result.update(
            {
                "status": "fail",
                "error": str(error),
                "failure_type": type(error).__name__,
                "completed_at_unix": int(time.time()),
            }
        )
        output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
