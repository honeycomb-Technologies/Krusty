#!/usr/bin/env python3
"""Resume only the Terra stage after a corrected provider-transport failure."""

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


def run(args: argparse.Namespace) -> dict[str, Any]:
    base_url = HARNESS.validate_candidate_base_url(args.base_url)
    output = args.root / "context-saturation-continuation-summary.json"
    HARNESS.require(output.is_file(), f"continuation summary was absent: {output}")
    result = json.loads(output.read_text())
    project_dir = args.root / "context-atlas"
    post_build = result.get("post_compaction_build", {})
    HARNESS.require(post_build.get("status") == "pass", "Grok post-compaction build was not proven")
    recovered = result.get("recovered_run", {})
    compaction = recovered.get("compaction", {})
    HARNESS.require(compaction.get("reason") == "auto", "automatic compaction proof was absent")
    HARNESS.require(
        compaction.get("estimated_tokens_before", 0) > compaction.get("estimated_tokens_after", 0),
        f"compaction did not reduce context: {compaction}",
    )
    api = HARNESS.KrustyApi(base_url, args.timeout)
    terra = HARNESS.select_stable_exact_model(
        api, args.terra_model, provider_id="open_a_i", timeout=60
    )
    HARNESS.require_session_model(api, args.terra_session_id, terra, "Terra resumed session")
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, args.terra_session_id), "Terra session before retry"
    )
    prior_trace = api.json_request(
        "GET", f"/api/sessions/{args.terra_session_id}/trace?limit=1000"
    )
    cursor = prior_trace.get("latest_sequence", 0)
    prior_summary = prior_trace.get("summary", {})
    HARNESS.require(
        prior_summary.get("last_stop_reason") == "provider_error",
        f"Terra session did not retain the transport failure: {prior_summary}",
    )
    result["terra_transport_recovery"] = {
        "failed_run_stop_reason": prior_summary.get("last_stop_reason"),
        "provider_failures": prior_summary.get("provider_failures"),
        "resumed_after_fix": True,
    }
    result["terra_session_id"] = args.terra_session_id
    result["status"] = "running"
    result.pop("error", None)
    result.pop("failure_type", None)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")

    events = api.chat(
        SATURATION.chat_payload(
            args.terra_session_id,
            "Retry the audit now that the provider transport is corrected. Audit and extend the "
            "existing Context Atlas project without replacing its architecture. Implement "
            "`report --index atlas-index.json --format json`; it must verify first and emit "
            "exactly `total_batches`, `total_records`, `origin_axiom`, and `integrity_status`. "
            f"origin_axiom is `{SATURATION.ORIGIN_AXIOM}` and integrity_status must be `ok`. "
            "Add tests, update README.md, create docs/TERRA_AUDIT.md with concrete findings, run "
            "all tests, demonstrate the report, and finish with `TERRA-HIGH-AUDIT-OK`.",
            terra,
            thinking="high",
        )
    )
    text = SATURATION.completed_text(events, "Terra High resumed audit")
    HARNESS.require("TERRA-HIGH-AUDIT-OK" in text, "Terra marker absent")
    trace, _ = HARNESS.wait_for_completed_trace_run(
        api,
        args.terra_session_id,
        "Terra High resumed trace",
        after_sequence=cursor,
        timeout=20,
    )
    effective_request = SATURATION.require_terra_high(trace, terra)
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
    index = SATURATION.load_index(project_dir)
    HARNESS.require(
        report_json["total_batches"] == len(index.get("batches", [])), "batch total drifted"
    )
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, args.terra_session_id), "Terra session after audit"
    )
    result["terra_high"] = {
        "status": "pass",
        "effective_request": effective_request,
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(events)],
        "usage": SATURATION.usage_summary(events),
        "tests": SATURATION.run_project_tests(project_dir),
        "report": report_json,
    }
    result["status"] = "pass"
    result["completed_at_unix"] = int(time.time())
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, indent=2, sort_keys=True))
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:3100")
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--terra-session-id", required=True)
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
