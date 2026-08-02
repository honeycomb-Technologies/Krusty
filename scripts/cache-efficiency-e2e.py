#!/usr/bin/env python3
"""Measure live warm-prefix cache behavior through an isolated Mitsuro server."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import time
from typing import Any


MEDIAN_TARGET_PCT = 93.85


def load_harness() -> Any:
    path = Path(__file__).with_name("harness-e2e-loop.py")
    spec = importlib.util.spec_from_file_location("mitsuro_cache_harness", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load harness helpers from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


HARNESS = load_harness()


def provider_calls(trace: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        event.get("payload", {})
        for event in trace.get("events", [])
        if event.get("event_type") == "provider_call"
        and event.get("call_kind") == "agent_loop"
    ]


def request_snapshots(trace: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        event.get("payload", {}).get("diagnostics", {})
        for event in trace.get("events", [])
        if event.get("event_type") == "provider_request_prepared"
    ]


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
        )
    }


def persist(root: Path, summary: dict[str, Any]) -> None:
    (root / "cache-efficiency-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n"
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    base_url = HARNESS.validate_candidate_base_url(args.base_url)
    HARNESS.require(not args.root.exists(), f"evaluation root already exists: {args.root}")
    args.root.mkdir(parents=True)
    project_dir = args.root / "workspace"
    project_dir.mkdir()
    api = HARNESS.MitsuroApi(base_url, args.timeout)
    model = HARNESS.select_stable_exact_model(
        api, args.model, provider_id=args.provider, timeout=60
    )
    response = api.json_request(
        "POST",
        "/api/sessions",
        {
            "title": f"Cache efficiency {args.provider} {args.model}",
            "model": model["id"],
            "model_key": model["key"],
            "project_dir": str(project_dir),
            "workspace_mode": "created",
            "session_type": args.session_type,
            "permission_mode": "autonomous",
        },
    )
    session_id = str(response.get("id") or "")
    HARNESS.require(bool(session_id), f"session create returned no id: {response}")

    anchor_lines = [
        f"CACHE-ANCHOR-{index:04d}-{hashlib.sha256(f'anchor:{index}'.encode()).hexdigest()}"
        for index in range(args.anchor_lines)
    ]
    anchor_sha256 = hashlib.sha256("\n".join(anchor_lines).encode()).hexdigest()
    summary: dict[str, Any] = {
        "status": "running",
        "base_url": base_url,
        "root": str(args.root),
        "session_id": session_id,
        "model": compact_model(model),
        "thinking": args.thinking,
        "session_type": args.session_type,
        "followup_calls_requested": args.calls,
        "anchor_lines": args.anchor_lines,
        "anchor_sha256": anchor_sha256,
        "calls": [],
    }
    persist(args.root, summary)

    trace_cursor = 0
    prompts = [
        "Treat this deterministic cache charter as stable reference context. "
        "Do not call tools. Reply exactly CACHE-PROBE-000 after reading it.\n\n"
        + "\n".join(anchor_lines)
    ]
    prompts.extend(
        f"Using the stable cache charter already in this session, do not call tools and reply exactly CACHE-PROBE-{index:03d}."
        for index in range(1, args.calls + 1)
    )

    all_calls: list[dict[str, Any]] = []
    all_snapshots: list[dict[str, Any]] = []
    microcompactions = 0
    full_compactions = 0
    for index, prompt in enumerate(prompts):
        payload = HARNESS.chat_payload(
            session_id,
            prompt,
            model if index == 0 else None,
            permission_mode="autonomous",
        )
        payload["thinking_enabled"] = args.thinking
        events = api.chat(payload)
        text = HARNESS.validate_stream(events, f"cache probe {index}", expect_tools=None)
        marker = f"CACHE-PROBE-{index:03d}"
        HARNESS.require(marker in text, f"probe {index} omitted {marker}")
        HARNESS.require(not HARNESS.tool_calls(events), f"probe {index} unexpectedly used tools")
        trace, _ = HARNESS.wait_for_completed_trace_run(
            api,
            session_id,
            f"cache probe {index} trace",
            after_sequence=trace_cursor,
            timeout=20,
        )
        trace_cursor = trace["latest_sequence"]
        calls = provider_calls(trace)
        snapshots = request_snapshots(trace)
        HARNESS.require(calls, f"probe {index} had no canonical provider-call trace")
        HARNESS.require(snapshots, f"probe {index} had no request snapshot")
        all_calls.extend(calls)
        all_snapshots.extend(snapshots)
        microcompactions += sum(
            event.get("event_type") == "microcompaction_applied"
            for event in trace.get("events", [])
        )
        full_compactions += sum(
            event.get("event_type") == "context_compacted"
            for event in trace.get("events", [])
        )
        call = calls[-1]
        summary["calls"].append(
            {
                "index": index,
                "input_tokens": call.get("input_tokens"),
                "cache_read_input_tokens": call.get("cache_read_input_tokens"),
                "cache_creation_input_tokens": call.get("cache_creation_input_tokens"),
                "completion_tokens": call.get("completion_tokens"),
                "continuation_mode": snapshots[-1].get("continuation_mode"),
                "cache_mode": snapshots[-1].get("cache_mode"),
                "cache_key_present": snapshots[-1].get("cache_key_present"),
            }
        )
        persist(args.root, summary)

    final_trace: dict[str, Any] = {}
    for _ in range(50):
        final_trace = api.json_request(
            "GET", f"/api/sessions/{session_id}/trace?limit=1000"
        )
        deduplicated = {
            call.get("provider_call_id"): call
            for call in provider_calls(final_trace)
            if call.get("provider_call_id")
        }
        if len(deduplicated) >= len(prompts):
            break
        time.sleep(0.1)
    all_calls = list(deduplicated.values())
    all_snapshots = request_snapshots(final_trace)

    usage_calls = [call for call in all_calls if call.get("usage_available") is True]
    hits = [
        call
        for call in usage_calls
        if isinstance(call.get("cache_read_input_tokens"), int)
        and call["cache_read_input_tokens"] > 0
    ]
    total_input = sum(int(call.get("input_tokens") or 0) for call in usage_calls)
    total_cache_read = sum(
        int(call.get("cache_read_input_tokens") or 0) for call in usage_calls
    )
    request_hit_pct = 100.0 * len(hits) / len(usage_calls)
    token_hit_pct = 100.0 * total_cache_read / total_input if total_input else 0.0
    HARNESS.require(
        len(all_calls) == len(prompts),
        f"expected {len(prompts)} provider calls, observed {len(all_calls)}",
    )
    HARNESS.require(len(usage_calls) == len(all_calls), "provider usage coverage was incomplete")
    HARNESS.require(
        request_hit_pct >= args.minimum_request_hit_pct,
        f"request cache hit {request_hit_pct:.2f}% was below {args.minimum_request_hit_pct:.2f}%",
    )
    HARNESS.require(
        all(snapshot.get("cache_key_present") for snapshot in all_snapshots),
        "one or more requests lacked a stable cache key",
    )
    HARNESS.require(full_compactions == 0, "warm-prefix probe unexpectedly fully compacted")

    summary.update(
        {
            "status": "pass",
            "completed_at_unix": int(time.time()),
            "provider_calls": len(all_calls),
            "usage_calls": len(usage_calls),
            "cache_hit_calls": len(hits),
            "request_hit_pct": round(request_hit_pct, 2),
            "token_hit_pct": round(token_hit_pct, 2),
            "input_tokens": total_input,
            "cache_read_input_tokens": total_cache_read,
            "microcompactions": microcompactions,
            "full_compactions": full_compactions,
            "median_target_pct": MEDIAN_TARGET_PCT,
        }
    )
    persist(args.root, summary)
    return summary


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:3180")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--thinking", default="off")
    parser.add_argument("--session-type", choices=("chat", "code"), default="chat")
    parser.add_argument("--calls", type=int, default=100)
    parser.add_argument("--anchor-lines", type=int, default=160)
    parser.add_argument("--minimum-request-hit-pct", type=float, default=MEDIAN_TARGET_PCT)
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args()
    if args.calls < 20:
        parser.error("--calls must be at least 20")
    if args.anchor_lines < 80:
        parser.error("--anchor-lines must be at least 80")
    return args


def main() -> int:
    args = parse_args()
    try:
        summary = run(args)
    except Exception as error:
        args.root.mkdir(parents=True, exist_ok=True)
        try:
            failure = json.loads(
                (args.root / "cache-efficiency-summary.json").read_text()
            )
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
        persist(args.root, failure)
        raise
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
