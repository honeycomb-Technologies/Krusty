#!/usr/bin/env python3
"""Finish the Terra High audit on an already compacted saturation project."""

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
    spec = importlib.util.spec_from_file_location("context_saturation_existing", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load saturation helpers from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


SAT = load_saturation()
HARNESS = SAT.HARNESS


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--terra-model", default="gpt-5.6-terra")
    parser.add_argument("--timeout", type=float, default=1200)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    summary_path = args.root / "context-saturation-summary.json"
    HARNESS.require(summary_path.is_file(), f"saturation summary absent: {summary_path}")
    summary = json.loads(summary_path.read_text())
    turns = summary.get("grok_turns", [])
    HARNESS.require(
        any(turn.get("compaction_in_trace", 0) > 0 for turn in turns),
        "Grok compaction proof absent",
    )
    HARNESS.require(
        summary.get("post_compaction_continuity", {}).get("status") == "pass",
        "post-compaction continuity proof absent",
    )
    HARNESS.require(
        summary.get("post_compaction_build", {}).get("status") == "pass",
        "post-compaction build proof absent",
    )
    project_dir = args.root / "context-atlas"
    api = HARNESS.MitsuroApi(
        HARNESS.validate_candidate_base_url(args.base_url), args.timeout
    )
    terra = HARNESS.select_stable_exact_model(
        api, args.terra_model, provider_id="open_a_i", timeout=60
    )
    session_id = SAT.create_session(
        api, project_dir, terra, "Terra High compacted Context Atlas audit"
    )
    prompt = f"""Audit and extend the existing Context Atlas project without replacing its architecture.

Implement `report --index atlas-index.json --format json`. It must verify first and emit exactly these top-level fields: `total_batches`, `total_records`, `origin_axiom`, `integrity_status`. The origin axiom is `{SAT.ORIGIN_AXIOM}` and integrity_status must be `ok`. Add meaningful tests, update README.md, create docs/TERRA_AUDIT.md with concrete findings, run the full suite, demonstrate the report, and finish with `TERRA-HIGH-AUDIT-OK`."""
    events = api.chat(
        SAT.chat_payload(session_id, prompt, terra, thinking="high")
    )
    text, recovered_errors = SAT.completed_text_with_recovered_tool_errors(
        events, "Terra High compacted-project audit"
    )
    HARNESS.require("TERRA-HIGH-AUDIT-OK" in text, "Terra marker absent")
    trace, _ = HARNESS.wait_for_completed_trace_run(
        api, session_id, "Terra High compacted-project trace", timeout=20
    )
    effective = SAT.require_terra_high(trace, terra)
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
    expected = {"total_batches", "total_records", "origin_axiom", "integrity_status"}
    HARNESS.require(set(report_json) == expected, f"report schema drifted: {report_json}")
    HARNESS.require(report_json["origin_axiom"] == SAT.ORIGIN_AXIOM, "axiom drifted")
    HARNESS.require(report_json["integrity_status"] == "ok", "integrity failed")
    HARNESS.require(
        report_json["total_batches"] == len(summary.get("batches", [])),
        "batch total drifted",
    )
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, session_id), "Terra compacted-project session"
    )
    summary["terra_session_id"] = session_id
    summary["terra_high"] = {
        "status": "pass",
        "effective_request": effective,
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(events)],
        "recovered_tool_errors": recovered_errors,
        "usage": SAT.usage_summary(events),
        "tests": SAT.run_project_tests(project_dir),
        "report": report_json,
    }
    summary["status"] = "pass"
    summary["completed_at_unix"] = int(time.time())
    summary.pop("error", None)
    summary.pop("failure_type", None)
    SAT.persist_summary(args.root, summary)
    print(json.dumps(summary["terra_high"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
