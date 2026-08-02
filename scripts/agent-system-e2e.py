#!/usr/bin/env python3
"""Live provider proof for Mitsuro's dynamic agent selection and delegated builds."""

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


def load_module(filename: str, name: str) -> Any:
    path = Path(__file__).with_name(filename)
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


HARNESS = load_module("harness-e2e-loop.py", "mitsuro_agent_harness")
GROK = load_module("grok-core-behavior.py", "mitsuro_grok_contract")
SATURATION = load_module("context-saturation-e2e.py", "mitsuro_saturation_contract")
AcceptanceFailure = HARNESS.AcceptanceFailure
require = HARNESS.require


def write_fixture(project: Path) -> None:
    project.mkdir(parents=True, exist_ok=False)
    (project / "LOOKUP.txt").write_text("DIRECT-LOOKUP-OK\n")
    (project / "README.md").write_text(
        "# Telemetry Report\n\n"
        "Build a dependency-free Python telemetry report with metrics.py and renderer.py.\n"
    )
    (project / "tests").mkdir()
    (project / "tests" / "test_report.py").write_text(
        "import unittest\n\n"
        "from metrics import summarize\n"
        "from renderer import render_markdown\n\n"
        "class ReportTests(unittest.TestCase):\n"
        "    def test_summary(self):\n"
        "        self.assertEqual(summarize([2, 4, 6]), {'count': 3, 'total': 12, 'mean': 4.0})\n\n"
        "    def test_render(self):\n"
        "        text = render_markdown({'count': 2, 'total': 10, 'mean': 5.0})\n"
        "        self.assertIn('Count: 2', text)\n"
        "        self.assertIn('Mean: 5.0', text)\n\n"
        "if __name__ == '__main__':\n"
        "    unittest.main()\n"
    )
    subprocess.run(["git", "init", "-q"], cwd=project, check=True)
    subprocess.run(["git", "add", "."], cwd=project, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Mitsuro Eval",
            "-c",
            "user.email=eval@localhost",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "evaluation fixture",
        ],
        cwd=project,
        check=True,
    )


def create_session(api: Any, project: Path, model: dict[str, Any], title: str) -> str:
    return HARNESS.create_session(
        api, project, model, title, permission_mode="autonomous"
    )


def payload(
    session_id: str,
    prompt: str,
    model: dict[str, Any],
    provider: str,
) -> dict[str, Any]:
    value = HARNESS.chat_payload(
        session_id, prompt, model, permission_mode="autonomous"
    )
    value["thinking_enabled"] = "high" if provider == "open_a_i" else "off"
    return value


def arguments(call: dict[str, Any]) -> dict[str, Any]:
    value = call.get("arguments")
    if isinstance(value, dict):
        return value
    if isinstance(value, str):
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError:
            return {}
        return decoded if isinstance(decoded, dict) else {}
    return {}


def agent_calls(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [call for call in HARNESS.tool_calls(events) if call.get("name") == "agent"]

def validate_recovering_build(
    events: list[dict[str, Any]], label: str
) -> tuple[str, list[dict[str, Any]]]:
    fatal = [
        event
        for event in events
        if event.get("type") in {"error", "server_tool_error", "lagged"}
    ]
    require(not fatal, f"{label}: terminal error events: {fatal}")
    recovered = [
        event
        for event in events
        if event.get("type") == "tool_result" and event.get("is_error") is True
    ]
    filtered = [
        {**event, "is_error": False} if event in recovered else event
        for event in events
    ]
    text = HARNESS.validate_stream(filtered, label, expect_tools=True)
    fingerprints = [
        hashlib.sha256(str(event.get("output", "")).encode()).hexdigest()
        for event in recovered
    ]
    require(
        len(fingerprints) == len(set(fingerprints)),
        f"{label}: repeated an unchanged failing tool result: {fingerprints}",
    )
    return text, recovered



def runtime_proof(
    events: list[dict[str, Any]],
    api: Any,
    session_id: str,
    model: dict[str, Any],
    label: str,
    provider: str,
) -> dict[str, Any]:
    trace, summary = HARNESS.wait_for_completed_trace_run(
        api, session_id, label, after_sequence=0, timeout=30
    )
    require(summary.get("last_stop_reason") == "completed", f"{label}: {summary}")
    runtime = GROK.validate_runtime_contract(events, trace, model, label)
    if provider == "open_a_i":
        effective = SATURATION.require_terra_high(trace, model)
        runtime["terra_effective_request"] = effective
    return {"trace_summary": summary, "runtime": runtime}


def run_no_delegate(
    api: Any,
    project: Path,
    model: dict[str, Any],
    provider: str,
) -> dict[str, Any]:
    session_id = create_session(api, project, model, f"{provider} direct lookup")
    events = api.chat(
        payload(
            session_id,
            "Read LOOKUP.txt and return its exact one-line value. This is a single known-file lookup; do not delegate, do not run Bash, and finish immediately.",
            model,
            provider,
        )
    )
    text = HARNESS.validate_stream(events, f"{provider} direct lookup", expect_tools=True)
    calls = HARNESS.tool_calls(events)
    require("DIRECT-LOOKUP-OK" in text, f"{provider}: lookup answer drifted: {text!r}")
    require(not agent_calls(events), f"{provider}: simple lookup delegated: {calls}")
    require(
        all(call.get("name") == "read" for call in calls),
        f"{provider}: simple lookup used unexpected tools: {calls}",
    )
    proof = runtime_proof(
        events, api, session_id, model, f"{provider} direct lookup trace", provider
    )
    return {
        "session_id": session_id,
        "tool_calls": [call.get("name") for call in calls],
        "response_sha256": hashlib.sha256(text.encode()).hexdigest(),
        **proof,
    }


def run_delegated_build(
    api: Any,
    project: Path,
    model: dict[str, Any],
    provider: str,
) -> dict[str, Any]:
    session_id = create_session(api, project, model, f"{provider} delegated build")
    prompt = """Implement the telemetry report described by README.md and make the existing tests pass.

The metrics and renderer modules are independent, so delegate their implementation as parallel build components through the agent tool. After delegated work returns, inspect the produced files, integrate any gaps yourself, and run the test suite once. Do not repeatedly run an unchanged command. Finish with the marker DELEGATED-BUILD-OK and a concise account of what the child agents produced."""
    events = api.chat(payload(session_id, prompt, model, provider))
    text, recovered_tool_errors = validate_recovering_build(
        events, f"{provider} delegated build"
    )
    calls = HARNESS.tool_calls(events)
    delegated = agent_calls(events)
    require(delegated, f"{provider}: substantial independent build never delegated: {calls}")
    require(
        any(arguments(call).get("action", "spawn") == "spawn" for call in delegated),
        f"{provider}: agent calls never spawned work: {delegated}",
    )
    require("DELEGATED-BUILD-OK" in text, f"{provider}: build marker absent: {text!r}")

    idle = HARNESS.wait_for_session_idle(api, session_id, timeout=120)
    runs = idle.get("recent_delegated_runs")
    require(isinstance(runs, list) and runs, f"{provider}: no durable delegated runs: {idle}")
    require(
        any(run.get("stage") in {"complete", "degraded"} for run in runs),
        f"{provider}: delegated run did not produce usable terminal evidence: {runs}",
    )
    for filename in ("metrics.py", "renderer.py"):
        require((project / filename).is_file(), f"{provider}: delegated build omitted {filename}")

    completed = subprocess.run(
        [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"],
        cwd=project,
        text=True,
        capture_output=True,
        timeout=60,
        check=False,
    )
    require(
        completed.returncode == 0,
        f"{provider}: external project tests failed:\n{completed.stdout}\n{completed.stderr}",
    )
    proof = runtime_proof(
        events, api, session_id, model, f"{provider} delegated build trace", provider
    )
    return {
        "session_id": session_id,
        "agent_actions": [arguments(call).get("action", "spawn") for call in delegated],
        "agent_profiles": [
            arguments(call).get("profile") or arguments(call).get("agent_type")
            for call in delegated
        ],
        "tool_calls": [call.get("name") for call in calls],
        "delegated_runs": runs,
        "recovered_tool_errors": [
            {
                "id": event.get("id"),
                "output_sha256": hashlib.sha256(
                    str(event.get("output", "")).encode()
                ).hexdigest(),
            }
            for event in recovered_tool_errors
        ],
        "files": {
            name: hashlib.sha256((project / name).read_bytes()).hexdigest()
            for name in ("metrics.py", "renderer.py")
        },
        "external_tests": {
            "returncode": completed.returncode,
            "stdout_sha256": hashlib.sha256(completed.stdout.encode()).hexdigest(),
            "stderr_sha256": hashlib.sha256(completed.stderr.encode()).hexdigest(),
            "tail": (completed.stdout + completed.stderr).splitlines()[-8:],
        },
        "response_sha256": hashlib.sha256(text.encode()).hexdigest(),
        **proof,
    }


def select_models(api: Any, args: argparse.Namespace) -> list[tuple[str, dict[str, Any]]]:
    grok = HARNESS.select_stable_exact_model(
        api, args.grok_model, provider_id="grok", timeout=60
    )
    terra = HARNESS.select_stable_exact_model(
        api, args.terra_model, provider_id="open_a_i", timeout=60
    )
    models = [("grok", grok), ("open_a_i", terra)]
    if args.provider == "both":
        return models
    return [item for item in models if item[0] == args.provider]


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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--grok-model", default="grok-4.5")
    parser.add_argument("--terra-model", default="gpt-5.6-terra")
    parser.add_argument(
        "--provider",
        choices=("both", "grok", "open_a_i"),
        default="both",
        help="Run both providers or one focused provider proof.",
    )
    parser.add_argument("--timeout", type=float, default=1200)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    args.base_url = HARNESS.validate_candidate_base_url(args.base_url)
    args.root.mkdir(parents=True, exist_ok=False)
    api = HARNESS.MitsuroApi(args.base_url, args.timeout)
    summary: dict[str, Any] = {
        "status": "running",
        "base_url": args.base_url,
        "root": str(args.root),
        "started_at_unix": int(time.time()),
        "providers": {},
    }
    summary_path = args.root / "agent-system-summary.json"
    try:
        for provider, model in select_models(api, args):
            project = args.root / f"{provider}-telemetry-report"
            write_fixture(project)
            summary["providers"][provider] = {
                "model": compact_model(model),
                "no_delegate": run_no_delegate(api, project, model, provider),
                "delegated_build": run_delegated_build(
                    api, project, model, provider
                ),
            }
        summary["status"] = "pass"
        summary["completed_at_unix"] = int(time.time())
        summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True))
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0
    except Exception as error:
        summary["status"] = "fail"
        summary["failure_type"] = type(error).__name__
        summary["error"] = str(error)
        summary["completed_at_unix"] = int(time.time())
        summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True))
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
