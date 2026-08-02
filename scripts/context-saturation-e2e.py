#!/usr/bin/env python3
"""Saturate one live Mitsuro project through compaction, then test Terra High."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time
from typing import Any


ORIGIN_AXIOM = "atlas-preserves-first-batch-after-compaction"


def _load_harness() -> Any:
    path = Path(__file__).with_name("harness-e2e-loop.py")
    spec = importlib.util.spec_from_file_location("mitsuro_harness_e2e", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load harness helpers from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


HARNESS = _load_harness()


def persist_summary(root: Path, summary: dict[str, Any]) -> None:
    path = root / "context-saturation-summary.json"
    path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")


def corpus_batch(batch: int, minimum_chars: int) -> dict[str, Any]:
    """Create deterministic, structured, non-repeating NDJSON pressure."""
    if batch < 1:
        raise ValueError("batch must be positive")
    if minimum_chars < 1_000:
        raise ValueError("minimum_chars must be at least 1000")

    lines: list[str] = []
    rendered_chars = 0
    sequence = 0
    while rendered_chars < minimum_chars:
        seed = f"context-atlas:{batch}:{sequence}".encode()
        digest = hashlib.sha256(seed).hexdigest()
        dependency = hashlib.sha256(b"depends:" + seed).hexdigest()[:24]
        record = {
            "batch": batch,
            "sequence": sequence,
            "record_id": f"B{batch:03d}-R{sequence:06d}-{digest[:12]}",
            "topic": f"topic-{sequence % 97:02d}",
            "dependency": f"B{max(1, batch - 1):03d}-{dependency}",
            "decision": (
                ORIGIN_AXIOM
                if batch == 1 and sequence == 0
                else f"decision-{digest}-{hashlib.sha256(digest.encode()).hexdigest()}"
            ),
            "acceptance": f"sha256:{digest}:sequence:{sequence}:batch:{batch}",
        }
        line = json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n"
        lines.append(line)
        rendered_chars += len(line)
        sequence += 1

    text = "".join(lines)
    return {
        "batch": batch,
        "text": text,
        "characters": len(text),
        "records": sequence,
        "sha256": hashlib.sha256(text.encode()).hexdigest(),
        "first_record_id": json.loads(lines[0])["record_id"],
        "last_record_id": json.loads(lines[-1])["record_id"],
    }


def compact_model(model: dict[str, Any]) -> dict[str, Any]:
    return {
        key: model.get(key)
        for key in (
            "id",
            "provider_id",
            "key",
            "catalog_source",
            "catalog_revision",
            "context_window",
            "max_output",
            "reasoning_control",
            "supported_reasoning_levels",
        )
    }


def trace_compactions(trace: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        event
        for event in trace.get("events", [])
        if event.get("event_type") == "context_compacted"
    ]


def sse_compactions(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [event for event in events if event.get("type") == "context_compacted"]


def usage_summary(events: list[dict[str, Any]]) -> dict[str, int]:
    totals = {
        "prompt_tokens": 0,
        "input_tokens": 0,
        "completion_tokens": 0,
        "reasoning_tokens": 0,
        "total_tokens": 0,
    }
    for event in events:
        if event.get("type") != "usage":
            continue
        for key in totals:
            value = event.get(key)
            if isinstance(value, int) and not isinstance(value, bool):
                totals[key] += value
    return totals


def run_project_tests(project_dir: Path) -> dict[str, Any]:
    required = [
        project_dir / "context_atlas.py",
        project_dir / "tests" / "test_context_atlas.py",
        project_dir / "README.md",
    ]
    missing = [str(path.relative_to(project_dir)) for path in required if not path.is_file()]
    HARNESS.require(not missing, f"Context Atlas artifacts missing: {missing}")
    completed = subprocess.run(
        [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"],
        cwd=project_dir,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    HARNESS.require(
        completed.returncode == 0,
        f"Context Atlas tests failed:\n{completed.stdout}\n{completed.stderr}",
    )
    return {
        "returncode": completed.returncode,
        "stdout_sha256": hashlib.sha256(completed.stdout.encode()).hexdigest(),
        "stderr_sha256": hashlib.sha256(completed.stderr.encode()).hexdigest(),
        "test_lines": [
            line
            for line in (completed.stdout + completed.stderr).splitlines()
            if line.startswith("test_") or line.startswith("Ran ") or line == "OK"
        ],
    }


def load_index(project_dir: Path) -> dict[str, Any]:
    path = project_dir / "atlas-index.json"
    HARNESS.require(path.is_file(), "atlas-index.json was not created")
    try:
        value = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise HARNESS.AcceptanceFailure(f"atlas-index.json was malformed: {error}") from error
    HARNESS.require(isinstance(value, dict), "atlas-index.json root must be an object")
    return value


def require_indexed_batch(
    project_dir: Path, batch: dict[str, Any], expected_batches: int
) -> dict[str, Any]:
    index = load_index(project_dir)
    serialized = json.dumps(index, sort_keys=True)
    for expected in (str(batch["batch"]), batch["sha256"], str(batch["records"])):
        HARNESS.require(expected in serialized, f"index omitted batch evidence {expected}")
    batches = index.get("batches")
    HARNESS.require(isinstance(batches, list), "index batches must be a list")
    HARNESS.require(len(batches) == expected_batches, f"index has {len(batches)} batches")
    return index


def request_snapshots(trace: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        event.get("payload", {}).get("diagnostics", {})
        for event in trace.get("events", [])
        if event.get("event_type") == "provider_request_prepared"
    ]


def require_terra_high(trace: dict[str, Any], terra: dict[str, Any]) -> dict[str, Any]:
    matching = [
        snapshot
        for snapshot in request_snapshots(trace)
        if snapshot.get("model_key") == terra.get("key")
    ]
    HARNESS.require(matching, "Terra trace did not use the selected exact model key")
    for snapshot in matching:
        effective = snapshot.get("effective_request", {})
        HARNESS.require(
            str(effective.get("reasoning_effort", "")).lower() == "high",
            f"Terra request did not resolve high reasoning: {effective}",
        )
        HARNESS.require(
            effective.get("thinking_enabled") is True,
            f"Terra request did not enable reasoning: {effective}",
        )
    return matching[-1]["effective_request"]


def create_session(api: Any, project_dir: Path, model: dict[str, Any], title: str) -> str:
    return HARNESS.create_session(
        api, project_dir, model, title, permission_mode="autonomous"
    )


def chat_payload(
    session_id: str,
    message: str,
    model: dict[str, Any] | None,
    *,
    thinking: str,
) -> dict[str, Any]:
    payload = HARNESS.chat_payload(
        session_id, message, model, permission_mode="autonomous"
    )
    payload["thinking_enabled"] = thinking
    return payload


def completed_text(events: list[dict[str, Any]], label: str) -> str:
    return HARNESS.validate_stream(events, label, expect_tools=None)


def completed_text_with_recovered_tool_errors(
    events: list[dict[str, Any]], label: str
) -> tuple[str, int]:
    """Accept a model-corrected tool failure without hiding product errors."""
    failed_indexes = [
        index
        for index, event in enumerate(events)
        if event.get("type") == "tool_result" and event.get("is_error") is True
    ]
    filtered = [
        {**event, "is_error": False} if index in failed_indexes else event
        for index, event in enumerate(events)
    ]
    text = HARNESS.validate_stream(filtered, label, expect_tools=None)
    if failed_indexes:
        last_failure = failed_indexes[-1]
        HARNESS.require(
            any(
                index > last_failure
                and event.get("type") == "tool_result"
                and event.get("is_error") is not True
                for index, event in enumerate(events)
            ),
            f"{label}: tool failure was not followed by successful tool evidence",
        )
    return text, len(failed_indexes)


def run(args: argparse.Namespace) -> dict[str, Any]:
    base_url = HARNESS.validate_candidate_base_url(args.base_url)
    HARNESS.require(not args.root.exists(), f"evaluation root already exists: {args.root}")
    args.root.mkdir(parents=True)
    project_dir = args.root / "context-atlas"
    project_dir.mkdir()
    (project_dir / "corpus").mkdir()
    api = HARNESS.MitsuroApi(base_url, args.timeout)

    grok = HARNESS.select_stable_exact_model(
        api, args.grok_model, provider_id=args.grok_provider, timeout=60
    )
    terra = HARNESS.select_stable_exact_model(
        api, args.terra_model, provider_id=args.terra_provider, timeout=60
    )
    HARNESS.require(
        isinstance(grok.get("context_window"), int) and grok["context_window"] >= 500_000,
        f"Grok context window was not saturation-grade: {compact_model(grok)}",
    )
    HARNESS.require(
        isinstance(terra.get("context_window"), int) and terra["context_window"] >= 272_000,
        f"Terra context window was unexpected: {compact_model(terra)}",
    )
    summary: dict[str, Any] = {
        "status": "running",
        "base_url": base_url,
        "root": str(args.root),
        "project_dir": str(project_dir),
        "grok": compact_model(grok),
        "terra": compact_model(terra),
        "batch_characters_requested": args.batch_chars,
        "max_batches": args.max_batches,
        "origin_axiom": ORIGIN_AXIOM,
        "grok_turns": [],
    }
    persist_summary(args.root, summary)

    grok_session = create_session(api, project_dir, grok, "Context saturation with Grok")
    summary["grok_session_id"] = grok_session
    trace_cursor = 0
    build_prompt = f"""Build a production-quality, dependency-free Python project named Context Atlas in this empty workspace.

Provide `context_atlas.py` with commands:
- `ingest CORPUS --index atlas-index.json`: validate UTF-8 NDJSON with one positive batch number and contiguous unique sequences, compute exact file SHA-256 and record count, and atomically upsert one batch without duplicating an identical batch.
- `verify --index atlas-index.json`: reread every indexed corpus path and fail if digest, record count, batch, or sequences changed.
- `query --index atlas-index.json --topic TOPIC`: emit deterministic JSON with the topic and matching record count.

The deterministic JSON index must contain a top-level `batches` list and `total_records`; each entry retains batch, path, sha256, and records. Preserve this origin axiom in README.md and module metadata: {ORIGIN_AXIOM}

Create meaningful tests in tests/test_context_atlas.py, create README.md with exact commands, run the tests, and report files and results. Do not install packages or start a background process."""
    build_events = api.chat(chat_payload(grok_session, build_prompt, grok, thinking="off"))
    build_text, build_recovered_tool_errors = completed_text_with_recovered_tool_errors(
        build_events, "Grok Context Atlas build"
    )
    build_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, grok_session, "Grok Context Atlas build trace", after_sequence=0, timeout=10
    )
    trace_cursor = build_trace["latest_sequence"]
    summary["grok_turns"].append(
        {
            "kind": "build",
            "assistant_text_sha256": hashlib.sha256(build_text.encode()).hexdigest(),
            "tool_calls": [call.get("name") for call in HARNESS.tool_calls(build_events)],
            "recovered_tool_errors": build_recovered_tool_errors,
            "usage": usage_summary(build_events),
        }
    )
    summary["initial_tests"] = run_project_tests(project_dir)
    persist_summary(args.root, summary)

    compaction_event: dict[str, Any] | None = None
    batches: list[dict[str, Any]] = []
    for batch_number in range(1, args.max_batches + 1):
        batch = corpus_batch(batch_number, args.batch_chars)
        corpus_path = project_dir / "corpus" / f"batch-{batch_number:03d}.ndjson"
        corpus_path.write_text(batch["text"])
        batches.append({key: value for key, value in batch.items() if key != "text"})
        relative = corpus_path.relative_to(project_dir)
        receipt = f"SATURATION-BATCH-{batch_number:03d}-INDEXED"
        pressure_prompt = f"""Continue the same Context Atlas project. The deterministic corpus shard below is also staged at `{relative}`.

Run `python3 context_atlas.py ingest --index atlas-index.json {relative}`, then run verify and the unit tests, and repair any real defect. Create `docs/checkpoints/batch-{batch_number:03d}.md` with batch {batch_number}, records {batch['records']}, SHA-256 {batch['sha256']}, first record {batch['first_record_id']}, and last record {batch['last_record_id']}. Do not copy the corpus into source or documentation. Finish with exact receipt `{receipt}`.

This structured fixture is included to exercise real provider context rather than padding. Exact SHA-256: {batch['sha256']}; exact records: {batch['records']}.

<context-atlas-corpus batch="{batch_number}">
{batch['text']}</context-atlas-corpus>
"""
        events = api.chat(chat_payload(grok_session, pressure_prompt, None, thinking="off"))
        text, recovered_tool_errors = completed_text_with_recovered_tool_errors(
            events, f"Grok saturation batch {batch_number}"
        )
        HARNESS.require(receipt in text, f"Grok response omitted {receipt}")
        trace, _ = HARNESS.wait_for_completed_trace_run(
            api,
            grok_session,
            f"Grok saturation batch {batch_number} trace",
            after_sequence=trace_cursor,
            timeout=20,
        )
        trace_cursor = trace["latest_sequence"]
        persistent_compactions = trace_compactions(trace)
        streamed_compactions = sse_compactions(events)
        if persistent_compactions:
            HARNESS.require(streamed_compactions, "durable compaction was absent from live SSE")
            candidate = persistent_compactions[-1]
            payload = candidate.get("payload", {})
            HARNESS.require(payload.get("reason") == "auto", f"not automatic: {candidate}")
            HARNESS.require(
                payload.get("estimated_tokens_before", 0) > payload.get("estimated_tokens_after", 0),
                f"compaction did not reduce context: {candidate}",
            )
            HARNESS.require(payload.get("compaction_count", 0) >= 1, "count absent")
            HARNESS.require(bool(payload.get("checkpoint_id")), "checkpoint absent")
            compaction_event = candidate

        index = require_indexed_batch(project_dir, batch, batch_number)
        summary["grok_turns"].append(
            {
                "kind": "saturation_batch",
                "batch": batch_number,
                "characters": batch["characters"],
                "records": batch["records"],
                "sha256": batch["sha256"],
                "tool_calls": [call.get("name") for call in HARNESS.tool_calls(events)],
                "usage": usage_summary(events),
                "compaction_in_sse": len(streamed_compactions),
                "compaction_in_trace": len(persistent_compactions),
                "recovered_tool_errors": recovered_tool_errors,
                "indexed_batches": len(index["batches"]),
                "tests": run_project_tests(project_dir),
            }
        )
        summary["batches"] = batches
        if compaction_event is not None:
            summary["compaction"] = compaction_event
        persist_summary(args.root, summary)
        if compaction_event is not None:
            break

    HARNESS.require(compaction_event is not None, "Grok did not compact before max_batches")
    summary["batches"] = batches
    summary["compaction"] = compaction_event

    continuity_marker = "POST-COMPACTION-CONTINUITY-OK"
    continuity_prompt = (
        "Without calling any tool, state the exact origin axiom established at the start "
        f"of this project, followed by `{continuity_marker}`. Do not explain."
    )
    continuity_events = api.chat(
        chat_payload(grok_session, continuity_prompt, None, thinking="off")
    )
    continuity_text = completed_text(continuity_events, "Grok continuity")
    HARNESS.require(not HARNESS.tool_calls(continuity_events), "continuity used a tool")
    HARNESS.require(ORIGIN_AXIOM in continuity_text, "origin axiom was lost")
    HARNESS.require(continuity_marker in continuity_text, "continuity marker absent")
    continuity_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, grok_session, "Grok continuity trace", after_sequence=trace_cursor, timeout=10
    )
    trace_cursor = continuity_trace["latest_sequence"]
    summary["post_compaction_continuity"] = {
        "status": "pass",
        "used_tools": False,
        "usage": usage_summary(continuity_events),
    }
    persist_summary(args.root, summary)

    first_record = batches[0]["first_record_id"]
    lineage_prompt = f"""Continue building Context Atlas after compaction. Add `lineage --index atlas-index.json --record-id RECORD_ID`, emitting deterministic JSON for the exact matching record and dependency and failing for an unknown record. Add tests, document it, run the suite, write docs/POST_COMPACTION.md, demonstrate record `{first_record}`, and finish with `POST-COMPACTION-BUILD-OK`."""
    lineage_events = api.chat(
        chat_payload(grok_session, lineage_prompt, None, thinking="off")
    )
    lineage_text = completed_text(lineage_events, "Grok post-compaction build")
    HARNESS.require("POST-COMPACTION-BUILD-OK" in lineage_text, "build marker absent")
    lineage_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, grok_session, "Grok post-compaction build trace", after_sequence=trace_cursor, timeout=10
    )
    trace_cursor = lineage_trace["latest_sequence"]
    lineage = subprocess.run(
        [sys.executable, "context_atlas.py", "lineage", "--index", "atlas-index.json", "--record-id", first_record],
        cwd=project_dir,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    HARNESS.require(lineage.returncode == 0, f"lineage failed: {lineage.stderr}")
    lineage_json = json.loads(lineage.stdout)
    HARNESS.require(first_record in json.dumps(lineage_json), "lineage omitted record")
    summary["post_compaction_build"] = {
        "status": "pass",
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(lineage_events)],
        "usage": usage_summary(lineage_events),
        "tests": run_project_tests(project_dir),
        "lineage_output": lineage_json,
    }
    persist_summary(args.root, summary)
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, grok_session), "Grok saturation session"
    )

    terra_session = create_session(api, project_dir, terra, "Terra High Context Atlas audit")
    summary["terra_session_id"] = terra_session
    terra_prompt = f"""Audit and extend the existing Context Atlas project without replacing its architecture.

Implement `report --index atlas-index.json --format json`. It must verify first and emit exactly these top-level fields: `total_batches`, `total_records`, `origin_axiom`, `integrity_status`. The origin axiom is `{ORIGIN_AXIOM}` and integrity_status must be `ok`. Add meaningful tests, update README.md, create docs/TERRA_AUDIT.md with concrete findings, run the full suite, demonstrate the report, and finish with `TERRA-HIGH-AUDIT-OK`."""
    terra_events = api.chat(
        chat_payload(terra_session, terra_prompt, terra, thinking="high")
    )
    terra_text, terra_recovered_tool_errors = completed_text_with_recovered_tool_errors(
        terra_events, "Terra High project audit"
    )
    HARNESS.require("TERRA-HIGH-AUDIT-OK" in terra_text, "Terra marker absent")
    terra_trace, _ = HARNESS.wait_for_completed_trace_run(
        api, terra_session, "Terra High trace", after_sequence=0, timeout=15
    )
    effective_request = require_terra_high(terra_trace, terra)
    report = subprocess.run(
        [sys.executable, "context_atlas.py", "report", "--index", "atlas-index.json", "--format", "json"],
        cwd=project_dir,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    HARNESS.require(report.returncode == 0, f"Terra report failed: {report.stderr}")
    report_json = json.loads(report.stdout)
    HARNESS.require(
        set(report_json) == {"total_batches", "total_records", "origin_axiom", "integrity_status"},
        f"Terra report schema drifted: {report_json}",
    )
    HARNESS.require(report_json["origin_axiom"] == ORIGIN_AXIOM, "Terra lost axiom")
    HARNESS.require(report_json["integrity_status"] == "ok", "Terra integrity failed")
    HARNESS.require(report_json["total_batches"] == len(batches), "Terra batch total drifted")
    HARNESS.require_clean_idle_state(
        HARNESS.wait_for_session_idle(api, terra_session), "Terra High session"
    )
    summary["terra_high"] = {
        "status": "pass",
        "tool_calls": [call.get("name") for call in HARNESS.tool_calls(terra_events)],
        "recovered_tool_errors": terra_recovered_tool_errors,
        "usage": usage_summary(terra_events),
        "effective_request": effective_request,
        "tests": run_project_tests(project_dir),
        "report": report_json,
    }
    summary["status"] = "pass"
    summary["completed_at_unix"] = int(time.time())
    persist_summary(args.root, summary)
    return summary


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:3100")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--grok-model", default="grok-4.5")
    parser.add_argument("--grok-provider", default="grok")
    parser.add_argument("--terra-model", default="gpt-5.6-terra")
    parser.add_argument("--terra-provider", default="open_a_i")
    parser.add_argument("--batch-chars", type=int, default=280_000)
    parser.add_argument("--max-batches", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=1_200.0)
    args = parser.parse_args()
    if args.batch_chars < 100_000:
        parser.error("--batch-chars must be at least 100000")
    if args.max_batches < 2:
        parser.error("--max-batches must be at least 2")
    return args


def main() -> int:
    args = parse_args()
    summary_path = args.root / "context-saturation-summary.json"
    try:
        summary = run(args)
    except Exception as error:
        args.root.mkdir(parents=True, exist_ok=True)
        try:
            failure = json.loads(summary_path.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            failure = {}
        failure.update(
            {
                "status": "fail",
                "error": str(error),
                "failure_type": type(error).__name__,
                "completed_at_unix": int(time.time()),
            }
        )
        persist_summary(args.root, failure)
        raise
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
