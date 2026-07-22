# Core evaluation and live Grok acceptance

Krusty's core gate has two deliberately different layers:

1. A deterministic, provider-neutral scenario corpus runs on every change. It
   proves decisions, counters, side-effect accounting, loop convergence, and
   trace shape without credentials, network variance, or provider spend.
2. The live Grok 4.5 acceptance runs against an exact server build in a
   disposable workspace. It proves the real catalog, auth, request encoder,
   streaming parser, orchestrator, tools, persistence, SSE, and process
   registry together.

Passing only one layer is not release proof. The deterministic corpus cannot
prove Grok's live wire behavior, and a successful live run cannot reliably
exercise every malformed or adversarial branch.

## Deterministic scenario corpus

Run:

```bash
bash scripts/core-eval.sh deterministic
```

The fixture-backed runner lives in
`crates/krusty-core/tests/support/scripted_scenario.rs`. Fixtures live under
`crates/krusty-core/tests/fixtures/scripted_scenarios/` and cover:

- a build workflow with one observed contract, one mutation, one validation,
  and a typed completion;
- a read-only audit that gathers new cited evidence and performs zero writes;
- a progressive 60-tool-turn audit, proving useful work is not stopped at the
  former 50-turn boundary;
- Grok-style cosmetic Bash variations with one semantic intent;
- successful no-op mutations and repeated successful validations;
- bounded transient retry, malformed stream failure, empty completion
  recovery, and context-overflow compaction/retry;
- a resolved-run parity contract for server, TUI, and ACP.

Each report contains a runtime-trace-compatible sequence and these acceptance
metrics:

- terminal stop reason;
- provider calls and provider retries;
- semantic-empty and context-overflow recoveries;
- compactions;
- tool calls and tool errors;
- mutation attempts, material side effects, and duplicate side effects;
- evidence, state, and validation deltas;
- progress events, no-progress cycles, and maximum no-progress streak.

The trace validator requires contiguous sequence numbers, exactly one terminal
event, the terminal event last, and provider/tool totals equal to the report.

The fixture states semantic tool intent explicitly. The same integration test
also drives the production `ProgressLedger` with progressive work, successful
no-op mutations, and cosmetically different Grok Bash calls, proving that its
derived intent and progress deltas match the fixture oracle. Surface parity is
covered separately through resolved `RunSpec` contracts and full workspace
tests; the live lane remains the proof of real provider and transport behavior.

## Live Grok 4.5 provider smoke

The lightweight smoke calls the Krusty `AiClient` simple and streaming paths
without running tools:

```bash
KRUSTY_GROK_LIVE_MODEL=grok-4.5 \
  bash scripts/core-eval.sh grok-provider-smoke
```

It requires a real X/Grok credential already available through Krusty's
credential store. It may consume quota. A pass proves both calls returned the
required markers; it does not prove agent-loop behavior.

## Full live Grok 4.5 acceptance on Honey

Use the exact candidate commit in an isolated clean worktree. Do not repoint or
restart the production service merely to run this gate. Start a candidate
server on a separate loopback port, then record its source and health before
testing:

```bash
export PATH="$HOME/.bun/bin:$PATH"
cd /path/to/clean/candidate-worktree
git status --short
git rev-parse HEAD
cargo build --release -p krusty
export KRUSTY_EVAL_PORT=3100
export KRUSTY_EVAL_WORKING_DIR=/home/burgess/Work/krusty-evals/workspace-<candidate-sha>
export KRUSTY_EVAL_DATABASE_PATH=/home/burgess/Work/krusty-evals/state-<candidate-sha>/krusty.db
cargo run --release -p krusty-server --example eval_server
```

The evaluation entrypoint bypasses the installed server PID file, uses the
explicit disposable database and workspace above, and still resolves the real
Honey credential store and model catalog. It also disables shared Mako daemon
discovery, persistent remote-access state, project MCP auto-connect, global
plugin/hook/skill contributions, push initialization, terminal access, and
unneeded administrative API routes. The only intentionally shared mutable
resource is Grok credential refresh state, because the live provider test must
use Honey's real credential. This prevents an evaluation command from silently
reusing production, touching the production control plane, or writing test
sessions into production's database.

In a second shell:

```bash
export PATH="$HOME/.bun/bin:$PATH"
cd /path/to/clean/candidate-worktree
curl --fail --silent http://127.0.0.1:3100/health
KRUSTY_BASE_URL=http://127.0.0.1:3100 \
KRUSTY_EVAL_ROOT=/home/burgess/Work/krusty-evals/grok-4.5-<candidate-sha> \
KRUSTY_GROK_LIVE_MODEL=grok-4.5 \
KRUSTY_EVAL_CYCLES=3 \
KRUSTY_EVAL_TIMEOUT=900 \
  bash scripts/core-eval.sh grok-live-e2e
```

`KRUSTY_EVAL_ROOT` must be a new disposable directory. The runner creates the
directory, requires an explicit loopback candidate URL, and refuses port 3000.
It first runs two focused core-behavior lanes:

- a supervised project audit restricted to read/grep/glob whose complete
  file, directory, symlink, mode, timestamp, size, and content snapshot is
  identical before and after, proving useful read-only evidence with zero
  executable mutation surface; both SSE and durable traces must report exactly
  those three advertised tool names;
- an adversarial sequence of cosmetically different Grok Bash calls, proving
  that canonical intent reaches replan or typed loop termination within the
  semantic policy bound rather than repeating forever.

Both lanes submit the exact catalog `ModelKey`, require an unlimited resolved
interactive budget, and compare every redacted provider-request snapshot in SSE
with the durable trace. The runner then builds and tests a dependency-free HTTP
project through the real agent, verifies live artifacts and endpoints, checks
model continuity and durable runtime traces, and exercises resilience lanes.
It writes:

- `core-behavior/core-behavior-summary.json` for the read-only and convergence
  proof;
- `acceptance-attempts.jsonl` for every attempt;
- `acceptance-summary.json` for clean cycles;
- `acceptance-final-summary.json` for the authoritative terminal status;
- per-cycle artifacts and failure evidence;
- `resilience/resilience-summary.json` when resilience lanes run.

A successful invocation requires three consecutive clean cycles. Narrowly
classified upstream 429/502/503/504 failures may consume the explicit external
retry budget; product, auth, configuration, tool, and ambiguous failures remain
terminal and cannot be hidden by retries.

Every clean cycle kills its harness-tracked demo server and proves the preview
port is quiet. Retaining the last demo process requires the explicit
`--retain-final-process` opt-in and is not used by the release gate. Stop the
isolated candidate server after collecting evidence and verify port 3100 is
quiet.

## Release evidence checklist

Keep these together for the candidate:

- candidate Git SHA and `git status --short` output;
- candidate binary version/hash and `/health` response;
- Grok catalog entry and resolved transport for `grok-4.5`;
- deterministic test output;
- provider-smoke output;
- `acceptance-final-summary.json` with `status: pass`;
- the three clean cycle results and resilience summary;
- final-process cleanup confirmation;
- any provider request IDs, with credentials and authorization headers
  redacted.
