# Death-loop remediation plan (2026-07-31)

> Historical pre-migration record: prior crate and service identifiers below
> are preserved exactly as observed on 2026-07-31.

Goals from forensic audit of Mitsuro sessions 26102, 06c5, cf41, d666.

## Goals

### G1 — Progress evidence fidelity (P0)
- Hash bash outcomes via history `output_preview` (not constant summary).
- No-progress telemetry must fingerprint rejected intents when accepted evidence is empty.
- Tests through history-shaped envelopes.

### G2 — Observational CI/status classification (P0)
- Proven read-only: `gh run view|list|watch` (read-only flags), status-only.
- Collapse poll presentation noise in `semantic_bash_signature` (`gh` jq/tail caps, git fetch banners).

### G3 — Background process completion wake (P0)
- Bind `session_id` on background bash spawns.
- On terminal process status, durable `queue_pending_steering` + live Steer when run active.
- Idempotent one-wake-per-terminal-transition.

### G4 — Soft interactive budget (P1)
- Unlimited interactive runs inject strategy/replan soft pressure at high turn counts (warn, not hard kill by default).

### G5 — Mobile soft-wrap lag (P0 product)
- Word-aware / more aggressive wrap estimate so expansion starts by ~line 2.
- Unit tests for long-word and soft-wrap typing.
- Bump iOS `buildNumber` for TestFlight after code lands.

### G6 — Verification
- Unit/integration tests green for krusty-core progress/shell/process.
- Deno composerGrowth tests.
- Live server rebuild + smoke: health, background sleep→wake path, progress unit tests.

## Success criteria
1. Repeated `git status` with *different* porcelain content yields *different* bash evidence keys after history packaging.
2. `gh run view 123` is Observe-class, not Mutate.
3. Background `sleep 1` with session_id produces pending steering on completion.
4. Soft-wrap estimate expands earlier for realistic iOS widths.
5. Server binary updated and `krusty-serve` healthy.

## Non-goals
- Full multi-agent wake graph.
- New model-facing await APIs.
- Low hard global max_turns.
