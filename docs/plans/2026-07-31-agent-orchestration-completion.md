# Goal: Agent orchestration completion (agnostic children + death-loop solid)

> Historical pre-migration record: prior command, service, database, and release
> path identifiers below are preserved exactly as observed on 2026-07-31.

**Status:** in progress (implementation started 2026-07-31)

**Created:** 2026-07-31
**Runtime authority at plan time:** `krusty serve` PID 216212, binary
`~/.local/bin/.krusty-releases/death-loop-remediation-20260731-074236/krusty`
(v0.9.20, matches `target/release/krusty`)
**Source audit:** live sessions in `~/.krusty/krusty.db` + Codex/Grok reference trees

---

## Product goal (one sentence)

Make Code mode an **orchestrating parent** that directs **agnostic, named child agents**
with free-form instructions, wakes on **child and background-job completion** instead of
polling, and **converges** (synthesize or ask) instead of thrashing.

---

## Competitive verification (why this is proper)

### Codex (official docs + harness tree `harness-review-20260721/codex`)

Codex subagents match the malleable-child model in substance:

| Codex behavior | Implication for Mitsuro |
|----------------|-------------------------|
| Parent spawns a **child thread** with a **task message** | Child is directed by parent prompt, not a fixed specialty binary |
| Identity uses **task name / nickname** | Name comes from the job, not only from a type enum |
| Optional `agent_type` is a **config layer** (`name` + `developer_instructions` in TOML) | Presets are instruction packs, not separate runtimes |
| Built-ins (`default`, `worker`, `explorer`) are thin presets | Optional shortcuts; core is still “spawn + instructions” |
| Parent **waits / steers / closes**; completion is **notified** (`subagent_notification`) | Event-driven return, no parent poll loops |
| Parent keeps **requirements + decisions**; children return **summaries** | Avoids context pollution (exact Codex rationale) |
| Children **inherit** sandbox / permissions from parent | Governance ceiling stays on parent |

Codex docs state explicitly: keep the main agent focused on decisions; move noisy work off the main thread; return summaries. That is the same architecture you described.

**Not required:** four hard-coded engines (plan/verify/explore/build) as separate execution personalities with fixed multi-hundred-line system prompts.

### Grok Build (local reverse / codegen: `harness-review-20260721/grok-build`)

| Grok behavior | Implication |
|---------------|-------------|
| Base subagent prompt: *“focused worker delegated a specific task”* | Agnostic child by default |
| `role_instructions` / **persona** are **overlays** on the same worker | Malleable via instructions, not new engines |
| Built-in types (`general-purpose`, `explore`, `plan`) mainly set **tool capability** | Capability ceiling ≠ product persona |
| Parent gets a **summary** when child finishes | Same notification pattern as Codex |
| Background shell tasks are a **separate** wake path | Aligns with our process-completion wake |

Grok is slightly more “type + persona catalog” than pure ad-hoc, but the **core** is still: one worker runtime + parent-directed task + optional instruction overlays.

### Goose (harness review)

Ad-hoc `delegate(instructions: "...")` with optional `async: true` — pure parent-directed malleability; synthesis on parent. Confirms the minimal form of your design.

### Verdict

**Your model checks out.** Industry-proper setup:

```text
Parent orchestrates
  → spawn agnostic child(name, instructions, capability ceiling, bg?)
  → child works in own context
  → completion notify / wait
  → parent integrates (never re-does child digs)
```

Hard-coded Plan/Verify/Explore/Build **runtimes** in Mitsuro are a local overfit, not the competitor pattern.

---

## Current Mitsuro gap (evidence)

| Area | Intended | Today |
|------|----------|--------|
| Child agents | One agnostic runtime; parent names + instructs | One `agent` tool, but **4 execution forks** + hard-coded Plan/Verify prompts |
| Naming | From plan / task | Partially (`name`); messaging still `agent_type` / profile |
| Live Code usage | Parent delegates multi-scope work | Heavy sessions: **0 `delegated_runs`**; parent bash thrash |
| Process wake | Background jobs notify parent | **Shipped** in death-loop binary; model rarely uses it |
| Subagent wake | Child complete injects into parent | Events exist; underused / not productized as Codex-style notify |
| Death loops | Semantic stop + land | Guards fire (good); often **error stop**, no forced synthesis |
| Research spirals | Converge | 40-turn archaeology still possible; spiral-fix not on main |
| Bash gravity | Prefer dedicated tools | Bash-dominant dialect |

Key sessions: `cf41373d` (CI poll), `06c504d3` (archaeology), `6d527692` (post-remediation guard stop), `26102bf9` (mobile thrash).

---

## Success criteria (done means)

1. **Agnostic child**
   - Spawn API: `name` + `instructions` (prompt) + optional capability ceiling + `run_in_background`.
   - No user/model-facing requirement to pick plan/verify/explore/build as agent *kinds*.
   - Child system prompt is generic worker + **parent instructions only** (plus project AGENTS.md / policy).
   - Display and completion use the parent-chosen **name**.

2. **Event-driven wait**
   - Background shell completion → durable steer / live `LoopInput::Steer` (already present; must stay).
   - Background **child** completion → same class of notify (parent does not poll `status` in a loop).
   - Soft budget text prefers wake over poll.

3. **No death loops / solid convergence**
   - CI/status polls classified as observe; no-progress ledger trustworthy (non-empty evidence signatures).
   - Guard path: Warn → Replan → **forced synthesis turn** (not only `Error` + push).
   - Pure exploration cannot expand forever without Communicate/answer.
   - Bash pure file-read/search nudged toward dedicated tools.

4. **Parent tendency**
   - System/tool contract: multi-scope or long digs → spawn child; small one-file work → stay parent.
   - Live smoke: at least one Code session produces `delegated_runs` rows and parent-thin transcript.

5. **Verification on the running Honey binary**
   - Rebuild/install path documented; `/health` version; process wake smoke; child spawn smoke; guard stop lands with synthesis.

---

## Non-goals

- Full multi-agent UI redesign (beyond status by name).
- Renaming CLI/`krusty` identifiers.
- Hard global max_turns as the primary loop detector.
- Keeping Plan/Verify as permanent separate engines “for legacy.”
- Shipping spiral-fix branch wholesale without review (pull only needed pieces).

---

## Workstreams and steps

### W0 — Product contract (short, written first)

**Deliverable:** freeze the agent spawn contract in one place (this plan + tool schema/docs).

```text
agent spawn:
  name: string              # required for UI / completion (from plan)
  instructions: string      # required — full malleable mind
  capabilities?: [read|write|execute]  # request; parent ceiling wins
  run_in_background?: bool
  max_turns?: number
  # lifecycle: list | status | wait | message | interrupt | resume
```

**Deprecate as product surface:** `profile` enum of plan/verify/explore/build as *kinds*.
**Optional later:** named instruction templates (Codex-style custom agent TOML) as *presets*, not engines.

**Effect:** one mental model for parent and for engineers.

---

### W1 — Collapse to one agnostic child runtime

**Goal:** delete the four-engine product model; keep one child loop.

| Step | Work | Effect |
|------|------|--------|
| W1.1 | Single child execution path (today’s explore/build merged into one governed worker) | No plan/verify special prompts as engines |
| W1.2 | Capability ceiling only gates tools (read vs write vs execute) | “Read-only dig” vs “implement” is policy, not persona |
| W1.3 | Child base prompt: generic worker + parent `instructions` + project context | Truly malleable |
| W1.4 | Persist role as `child` / single delegated role (migrate planner/verifier → child) | Storage matches product |
| W1.5 | Tool description + parent system contract updated | Model stops shopping for specialist agents |
| W1.6 | Compatibility: map old `profile=explore|build|…` → capabilities + optional instruction appendix if needed | No silent break for old clients |

**Acceptance:** unit tests for spawn name/instructions; custom-looking job uses same runtime; no PlanConfig/VerifyConfig required for spawn.

---

### W2 — Completion notify (children + processes)

**Goal:** parent never burns turns polling “are you done?”

| Step | Work | Effect |
|------|------|--------|
| W2.1 | Keep process wake (`process_wake.rs`) + session_id binding on bg bash | CI/build detach works |
| W2.2 | Child background complete → durable pending steer + live Steer when run active (Codex-like notify) | Subagent path matches process path |
| W2.3 | Completion payload includes **name**, summary, success, delegated_run_id | UI/parent readable |
| W2.4 | Instruct model: on notify, **continue once**; do not re-poll that id | Tendency |
| W2.5 | Smoke: sleep bg bash wake; bg child wake (automated test + manual on port 3000) | Runtime proof |

**Acceptance:** traces show steer injection on complete; no multi-turn status poll for same process/run.

---

### W3 — Death-loop / progress / landing (finish remediation)

**Goal:** thrash stops *and* conversation lands.

| Step | Work | Effect |
|------|------|--------|
| W3.1 | Residual empty evidence signatures after history packaging (edit/bash `changed` / keys) | Progress ledger trustworthy |
| W3.2 | Observational CI/status classification stays tight (`gh`/`git` poll noise) | Already partly shipped; regression tests |
| W3.3 | On loop-guard Stop: **one forced synthesis turn** (tools off or read-only), then finish | User gets answer/blockers, not only error push |
| W3.4 | Earlier Observe-only / archaeology pressure (pull useful bits from spiral-fix if needed) | Research converges before turn 40 |
| W3.5 | Soft interactive budget: keep unlimited default; ensure warn/replan text aligns with wake+delegate | Visible pressure without hard kill |
| W3.6 | Bash gravity: when shell is pure file read/search, surface recommended dedicated tool | Cleaner evidence + less samey bash walls |

**Acceptance:** replay-class tests; live session cannot 100× bash CI poll; guard stop yields synthesis message in transcript.

---

### W4 — Parent tendency (use children)

**Goal:** Code mode actually behaves like an orchestrator on multi-scope work.

| Step | Work | Effect |
|------|------|--------|
| W4.1 | System prompt + agent tool prompt: when to spawn vs stay inline | Model has a clear rule |
| W4.2 | Optional: after N pure-observe turns with multi-dir digs, replan “spawn a named child or answer” | Nudges without hard fail |
| W4.3 | Scenario smoke scripts (see matrix below) | Proof, not hope |

---

### W5 — Build, deploy, verify on Honey

| Step | Work |
|------|------|
| W5.1 | `cargo test` focused agent/progress/process; then workspace gates proportional to change |
| W5.2 | Release build + install to `krusty-serve` release path (same pattern as death-loop-remediation) |
| W5.3 | Confirm `/proc/<pid>/exe`, `/health`, MD5 vs artifact |
| W5.4 | Smoke matrix on live server + DB traces |

---

## Scenario matrix (intended conversation shapes)

| Scenario | Expected transcript | Mechanisms |
|----------|---------------------|------------|
| Small fix | Parent read → edit → light validate → done | No child |
| Multi-area dig | Parent spawns named child(ren) with instructions; wait/notify; parent answer | Agnostic child + complete wake |
| Feature implement | Parent (or child named for task) implements under instructions; one validation arc | Child or parent write cap |
| Long CI/build | `run_in_background` → quiet → process complete steer → one status/read → conclude | Process wake |
| Stuck thrash | Guard warn → replan → stop → **synthesis** | Progress + landing |
| Parallel independent digs | Two+ named bg children → two notifies → parent integrate | Child wake |

---

## Dependency order

```text
W0 contract
  → W1 agnostic runtime (blocks correct product language)
  → W2 child notify (depends on single child lifecycle)
  → W3 death-loop landing (can parallel early with W1 tests)
  → W4 tendency (after tool contract stable)
  → W5 deploy + live verify
```

W3.1–W3.2 can start in parallel with W1 if needed; W3.3 should land after guard path is stable.

---

## Implementation notes (for whoever builds)

- Prefer **one** `execute_child` path over four `execute_*` specialists.
- Capabilities: parent `DelegationPolicy` remains the ceiling.
- `components` parallel build may remain as an **optimization** of the same child runtime (multiple children with different names/instructions), not a separate “BuildAgent” type.
- Do not reintroduce giant tool manuals in system prompt; schemas stay the tool contract.
- Keep UI history vs model history split (already correct).
- Measure success with **runtime_traces** + `delegated_runs` + journal, not vibes.

---

## Open decisions (defaults if unstated)

| Decision | Default |
|----------|---------|
| Keep optional named instruction presets (Codex TOML-like)? | **Later**; not required for v1 of this goal |
| Parallel multi-component build API | Parent spawns **N named children** (preferred) vs one spawn with components array |
| Forced synthesis on guard | Always on for interactive Code; Goals may block with reason |
| Chat mode | No child spawn required; thin path stays |

---

## Tracking checklist

- [ ] W0 contract frozen
- [ ] W1 agnostic child runtime
- [ ] W2 process + child completion notify
- [ ] W3 progress fidelity + synthesis landing + spiral pressure
- [ ] W4 parent tendency
- [ ] W5 live binary verified

---

## References

- Session audit (this thread): death-loop binary, heavy sessions, bash/CI patterns
- `docs/plans/2026-07-31-death-loop-remediation.md` (partially shipped)
- Codex: https://learn.chatgpt.com/docs/agent-configuration/subagents
- Local: `harness-review-20260721/codex`, `harness-review-20260721/grok-build`
- Commit: `8fcbae7` process wake; spiral-fix `8b8a929` not on main
