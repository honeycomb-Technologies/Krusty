# Mitsuro Unify → Staging Plan (Front-to-Back)

**Status:** approved goal / ready to execute (no merge until phase gates pass)  
**Date:** 2026-08-02  
**Authority sink:** `codex/release-staging-20260801` on `honeycomb-Technologies/Mitsuro`  
**Live production (do not thrash):** Honey install  
`~/.local/bin/.krusty-releases/agent-loop-preview-20260801-5aea43da-bdf5c518/`  
(`krusty serve` + `krusty-mako` against `~/.krusty`)

---

## 1. End-state goal (Definition of Done)

One sentence:

> **Everything across machines is merged cleanly onto one staging tip with zero unique work left only in dirty indexes or orphan worktrees; that tip fully replaces the legacy terminal with tui_v2, completes the Mitsuro/Hive rename (no shipping Krusty/Mako product identity), includes server/core updates, and is the single commit history every Mitsuro git (worktree, Codex, Grok, CI, Honey, Mac) pulls and builds.**

### DoD checklist

| # | Criterion | Done when |
|---|-----------|-----------|
| D1 | Single staging tip | `origin/codex/release-staging-20260801` (or successor) is the only authority; `git rev-parse HEAD` matches across machines after fetch |
| D2 | TUI full replace | Default CLI entry runs **tui_v2 only**; v1 TUI not default; archived or removed after confidence |
| D3 | Full rename live | Crates/bins/docs/packaging are Mitsuro/Hive; no product path still requires Krusty/Mako names for normal use |
| D4 | Server/core in | Agent-loop / server rewrite / schema behavior is **in git** on the tip (not only on a preview binary) |
| D5 | No loss | Every unique dirty tree is captured on a named branch, integrated, or proven redundant |
| D6 | Clients reconciled | Mobile + desktop deltas selectively in tip; no orphan “better” branches with unmerged unique commits |
| D7 | Git sync policy | All future work starts from that tip; worktrees are disposable; only pushed commits count |
| D8 | Clean index on authority checkout | Primary staging worktree `git status` empty after integrate |
| D9 | Validation | Workspace cargo check/test/clippy/fmt + mobile export (as applicable) green on tip |
| D10 | Preview optional | Private Honey preview built **from tip SHA only** (not dirty tree) |

**Out of scope for this goal:** force-merge to `main`, public release tag, production restart — those require explicit later approval.

---

## 2. Inventory (sources of truth)

### 2.1 Honey (multi-worktree)

| Path | Branch / HEAD | Dirty | Disposition |
|------|----------------|-------|-------------|
| `~/Work/krusty` | `codex/release-staging-20260801` @ `57e065b` | ~**1320** (identity rename + apps) | **Sink / Phase 1 snapshot** |
| `~/Work/krusty-agent-loop-validation-aaiGPu` | detached `57e065b` | ~**74** (mostly `crates/`) | **Phase 0 capture → Phase 2 integrate** |
| `~/Work/krusty-desktop-product-20260726` | `codex/desktop-product-20260726` clean | 0 | Phase 4 selective |
| `~/Work/krusty-release-staging-20260727-mobile` | clean, upstream gone | 0 | Phase 4 selective |
| `/tmp/krusty-spiral-fix-build` | detached `8b8a929` clean | 0 | Phase 5 retire after verify |
| Live install `agent-loop-preview-20260801-5aea43da-bdf5c518` | n/a | n/a | Reference behavior for core/server |

Remote: `origin → honeycomb-Technologies/Mitsuro.git`  
Layout: `mitsuro-cli|core|server|hive` (rename on dirty tree). **No tui_v2 on honey yet** (v1 `tui` only).

### 2.2 Mac (this session)

| Path | State | Disposition |
|------|--------|-------------|
| `/private/tmp/mitsuro-tui-v2-resume-20260731` | **Full tui_v2** (~83 rs / ~26k LOC), still `krusty-*` names; **gitdir broken** (Documents worktree) | **Phase 0 preserve at all costs** |
| `/Users/Jacob/Work/krusty` | `codex/wire-exploration-loop-guards-20260730`, almost clean, remote still Krusty | Align to Mitsuro remote + staging tip after unify |
| Orphan tmp worktrees (`krusty-mitsuro-tui-v2-plan-*`, mobile-report, beam-button) | Broken git links | Verify then drop after Phase 0 |

### 2.3 Critical gap

```
Honey staging:  mitsuro-* + massive rename dirty + v1 tui
Mac resume:     krusty-* + complete tui_v2 + broken git
Live server:    installed preview binary (core/server crucial)
```

Cannot “just merge” resume onto honey without **preserve → rename → integrate**.

---

## 3. Non-negotiables

1. **Never lose TUI v2 work** — capture named branch before any delete/rewrite.  
2. **Never lose agent-loop dirty** — unique core/server; capture before integrate.  
3. **Do not thrash live install** until tip is intentional and validated.  
4. **No wholesale merge** of archive/old rollup branches — audit commits.  
5. **No merge to `main`** until user approves after staging green.  
6. **Staging tip only** for Honey private previews.  
7. Each worktree is its own index — commit on source branch, then cherry into staging.

---

## 4. Phase plan (front to back)

### Phase 0 — Preserve (zero loss)

**Owner:** ops/orchestrator  
**Budget:** high (see §6)

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P0.1 | Repair or re-home Mac resume tree into a real git worktree of Mitsuro | Working `git status` | `git log -1` works |
| P0.2 | Commit/push **entire tui_v2 + related** as `codex/tui-v2-preserve-20260802` | Branch on origin | Diff size matches tree |
| P0.3 | On honey agent-loop validation: create branch, commit dirty | `codex/core-server-agent-loop-20260802` | dirty count → 0 |
| P0.4 | Record live install SHA/path + `main`/`staging` SHAs in this doc appendix | Snapshot table | Doc updated |
| P0.5 | Optional: `git bundle create` of preserve branches onto Honey + Mac | Bundle files | `git bundle verify` |

**Exit:** TUI and core/server unique work exist as **named, recoverable refs** independent of dirty indexes.

---

### Phase 1 — Freeze identity on staging

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P1.1 | On honey `codex/release-staging-20260801`, commit **all** ~1320 dirty as explicit snapshot | One or few commits, e.g. `chore(staging): Mitsuro identity migration snapshot` | `git status` clean |
| P1.2 | Push staging tip to `origin` | Remote tip updated | Mac can `fetch` same SHA |
| P1.3 | Document any intentional leftovers | Notes in PR/issue | None left silent |

**Exit:** Staging is clean, renamed Mitsuro layout is committed, still may run v1 TUI.

---

### Phase 2 — Core / server rewrite into staging

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P2.1 | Diff `codex/core-server-agent-loop-*` vs staging tip | Patch list | Reviewer sign-off |
| P2.2 | Integrate missing commits/patches (prefer logical commits) | Staging history | Compiles |
| P2.3 | Align with live preview behavior (delegated runs, leases, history policy, schema) | Notes of gaps | Smoke server |
| P2.4 | `cargo check --workspace` + targeted core tests | Green or known issues listed | Log |

**Exit:** Server/core “most crucial” work is **in git** on staging, not only in install.

---

### Phase 3 — TUI v2 full replace

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P3.1 | Port `codex/tui-v2-preserve-*` onto post-rename tree (`mitsuro-cli`) | Branch `codex/tui-v2-replace-20260802` | Builds under new crate names |
| P3.2 | Land tui_v2 sources + presentation/tool/projection work | Commits on staging | Unit tests tui_v2 |
| P3.3 | Switch CLI entry to tui_v2 only | Default binary path | `mitsuro` / `krusty` alias launches v2 |
| P3.4 | Archive or delete v1 `tui` default paths | Clean module graph | No dual entry |
| P3.5 | Tool envelope + line-number + panel quality from this session | Included in P3.2 | Manual smoke expand Read/Bash/Edit |

**Exit:** New terminal is the only TUI; legacy terminal not product default.

---

### Phase 4 — Clients selective reconcile

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P4.1 | Audit `desktop-product` vs staging; cherry missing intended | Staging commits | Desktop build notes |
| P4.2 | Audit `20260727-mobile` vs staging; cherry missing intended | Staging commits | Mobile export |
| P4.3 | Drop proven-redundant patches | Record | Cherry list |

**Exit:** No orphan client branch with unique intended work.

---

### Phase 5 — Global git sync

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P5.1 | Push all preserve/replace branches if not already | origin refs | `git ls-remote` |
| P5.2 | On **every** machine: `git fetch origin` + checkout/reset primary to staging tip | Same SHA | `rev-parse` table |
| P5.3 | Retire orphan worktrees after prove HEAD reachable on origin | `git worktree remove` | worktree list clean |
| P5.4 | Point Mac `Work/krusty` remote to Mitsuro if still Krusty | remote -v | fetch works |
| P5.5 | Policy note in AGENTS or CONTRIBUTING: all agents/worktrees start from staging tip | Doc commit | Reviewed |

**Exit:** Honey, Mac, GitHub, future Grok/Codex sandboxes all see the same tip.

---

### Phase 6 — Validation & freeze

| ID | Task | Output | Verify |
|----|------|--------|--------|
| P6.1 | Full required validation from AGENTS.md | Logs | All green |
| P6.2 | Optional Honey private preview **from tip SHA** | Preview URL/note | Matches tip |
| P6.3 | Staging freeze checklist signed | This doc §7 | User OK |
| P6.4 | **Stop** — main/release only on explicit later approval | — | — |

---

## 5. GitHub tracking (skills enabled)

Use GitHub Issues on **`honeycomb-Technologies/Mitsuro`** as the execution board.

### Parent epic

**Title:** `[Unify] Staging full migration — TUI v2 replace + rename + server/core`

**Body:** link this plan path; DoD §1; phases 0–6.

### Child issues (one per phase)

| Issue title | Phase |
|-------------|--------|
| `[Unify P0] Preserve TUI v2 + agent-loop unique work` | 0 |
| `[Unify P1] Snapshot identity rename on release-staging` | 1 |
| `[Unify P2] Integrate core/server agent-loop into staging` | 2 |
| `[Unify P3] Full tui_v2 replace (archive legacy TUI)` | 3 |
| `[Unify P4] Selective desktop + mobile reconcile` | 4 |
| `[Unify P5] Cross-machine git sync to single tip` | 5 |
| `[Unify P6] Validation freeze (no main yet)` | 6 |

Labels (create if missing): `unify`, `staging`, `tui-v2`, `identity`, `core`, `server`.

### Execution skill hooks

| Tool | Use |
|------|-----|
| GitHub MCP `issue_write` | Epic + phase issues |
| `pr-babysit` / `gh pr create` | After integrate branches exist |
| Grok workflow `mitsuro-unify-staging` | Max-budget orchestration |
| `/execute-plan` | Only if a PR-Plan DAG is produced from this doc later |

---

## 6. Max-budget execution setup

### 6.1 Grok workflow

**Name:** `mitsuro-unify-staging`  
**Path:** `.grok/workflows/mitsuro-unify-staging.rhai`  
**Agent budget:** **1024** (platform max)  
**Mode:** phased; human gates between P0→P1→P2→P3→P4→P5→P6  

Workflow phases map 1:1 to §4. Agents default:

- **read-only** inventory/diff/audit  
- **read-write** only on explicit preserve/integrate phases after `await_user`  
- **execute** for git capture, cargo test, status  

### 6.2 Human gates (hard)

| After | Require user before continue |
|-------|------------------------------|
| P0 | Confirm preserve branches on origin |
| P1 | Confirm identity snapshot OK |
| P2 | Confirm core/server integrate OK |
| P3 | Confirm TUI replace OK (manual smoke) |
| P5 | Confirm all machines show same SHA |
| P6 | Explicit approve freeze (main still separate) |

### 6.3 Parallelism policy

- **P0** capture lanes can parallelize (TUI preserve ‖ agent-loop preserve).  
- **P1–P3** sequential (rename → core → tui depends on layout).  
- **P4** desktop ‖ mobile audits parallel; integrates sequential.  
- **P5–P6** sequential.

### 6.4 Failure policy

- Preserve always fail-closed (retry until branch exists).  
- Integrate fail-closed on cargo red unless user waives.  
- Never delete worktrees until origin has the branch and user confirms.

---

## 7. Freeze checklist (sign-off)

- [x] P0 branches on `origin` — `codex/core-server-agent-loop-20260802`, `codex/tui-v2-replace-20260802`
- [x] Staging clean after P1 — identity snapshot landed (`7f558286` lineage)
- [x] Core/server from agent-loop in tip — `9a6d2599` merge + follow-up fixes
- [x] tui_v2 is default entry — `mitsuro-cli` `tui_v2::run()` only
- [x] No product Krusty/Mako names required for normal use — crates are `mitsuro-*` / `hive_*`
- [x] Desktop/mobile intended commits in tip — desktop land `7b0e3d9f` (mobile selective as needed)
- [x] Honey Mac GitHub same `rev-parse` — tip `c59770f0` (2026-08-02)
- [~] Validation green — **`cargo check` + `cargo test --workspace` green on tip**; `cargo clippy -D warnings` still red (~590 pre-existing CLI warnings); `cargo fmt --check` still dirty on unrelated files
- [ ] User: “freeze staging”
- [ ] User: “merge main” (later, not this goal)

---

## 8. Appendix — snapshot SHAs (fill during P0.4)

| Ref | SHA | Notes |
|-----|-----|-------|
| `main` (honey at plan time) | `c425a53` | |
| staging tip (honey at plan time) | `57e065b` | + dirty identity |
| staging tip (2026-08-02 P6) | `c59770f0` | includes markdown/hive test fixes |
| live install id | `agent-loop-preview-20260801-5aea43da-bdf5c518` | do not thrash |
| tui preserve/replace | `a24b8f8b` / `codex/tui-v2-replace-20260802` | on origin |
| core preserve branch | `47f9abed` / `codex/core-server-agent-loop-20260802` | on origin |

---

## 9. Run commands (when executing)

```bash
# Workflow (max budget)
# via Grok: workflow name=mitsuro-unify-staging agent_budget=1024
# or /workflow mitsuro-unify-staging

# Manual authority checks
git fetch origin
git rev-parse HEAD origin/codex/release-staging-20260801

# Required validation (staging tip)
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
# mobile as needed:
# cd apps/mobile && npx expo export --platform web
```

---

*This document is the single front-to-back plan for unify. Execution is gated; start with Phase 0 only unless user expands scope.*

---

## Appendix — Phase 4 reconcile record (2026-08-02)

### Desktop (`codex/desktop-product-20260726`)
- Landed missing `apps/desktop/ui` workstation surface + shell packaging notes onto staging.
- Renamed product plane `MakoPlane` → `HivePlane` and aligned package aliases to `@mitsuro/*`.
- Staging already had Tauri shell/identity compatibility; product UI was the unique gap.

### Mobile (`codex/release-staging-20260727-mobile`)
- Audit: no non-mako unique files missing on staging.
- Staging already contains Hive-renamed companion UI, mitsuro-diagnostics, and mobile perf/diagnostics suite from the identity snapshot.
- Mobile branch remaining tip deltas are **behind** identity rename (legacy `mako/` paths) and are intentionally **not** reintroduced.
- Disposition: **proven redundant for unique intended work** after identity snapshot; keep branch as historical reference only.

### Cherry list
| Source | Action | Result |
|--------|--------|--------|
| desktop-product `apps/desktop/ui` | checkout + Hive/Mitsuro rename | landed |
| desktop-product shell package/README | selective | landed / fixed JSON |
| mobile-branch unique non-identity | none | redundant |

---

## Appendix — Phase 6 freeze record (2026-08-02)

**Authority tip:** `9922454a` (`9922454a57d884922cedf98a3d2c080ebc78dcd6`) on `codex/release-staging-20260801`  
**Remote:** `origin` → `honeycomb-Technologies/Mitsuro`

### Validation (AGENTS required)

| Check | Result |
|-------|--------|
| `cargo check --workspace` | green |
| `cargo test --workspace --no-fail-fast` | green (all crates; ~2.4k unit tests) |
| `cargo clippy --workspace -- -D warnings` | green (legacy v1 `tui` allows dead_code; tui_v2 nits fixed) |
| `cargo fmt --all -- --check` | green |

### Stage disposition

| Phase | Status |
|-------|--------|
| P0 preserve | done — named branches on origin |
| P1 identity snapshot | done |
| P2 agent-loop core/server | done + schema migration 55 |
| P3 tui_v2 default | done |
| P4 desktop/mobile | done (desktop UI landed; mobile proven redundant) |
| P5 git sync | done — staging tip + preserve branches pushed; Mac remote retargeted to Mitsuro |
| P6 validation freeze | **done at tip above** |

### Explicit stop

- **No** merge to `main`
- **No** production restart / public release without separate approval
- Private Honey preview must be built from this tip SHA only if requested later

