---
name: quality-improve
description: >-
  Gated improvement loop that deepens modules, simplifies expression, and
  speeds hot paths without deleting dead code or changing behavior, design,
  or state. Use when the user says quality-improve, fire the quality loop,
  deepen modules, simplify without deleting, or write simple-and-advanced code.
disable-model-invocation: true
version: 1.0.0
tags:
  - quality
  - refactor
  - architecture
---

# Quality Improve

Write the deepest simple thing: a small typed interface, invariants proven
once at the edge, effects in a thin shell, no extra seams, no dead-code hunt.

This skill is the constitution and the loop. Do not start a clock. One sector,
one slice, critic, validate, ledger. Default authority is **propose-only**.

Read [reject-list.md](reject-list.md) before any edit. Read [reference.md](reference.md)
for the techniques. Use the fleet in `.cursor/agents/`. Record every tick in
[ledger.md](ledger.md).

## Authority

| Mode | When | Writes |
|---|---|---|
| `propose-only` | Default, and until readiness is green | No product code |
| `apply` | User said apply, and readiness is green | One approved slice, revert on fail |

Never invent apply mode. If the user says "fire it" without "apply", stay
propose-only.

## Always write this way

1. **Deep module** — lots of behavior behind a small interface. Shallow wrappers
   get inlined. Complexity that callers would otherwise repeat stays behind the
   interface.
2. **Simple ≠ easy** — unbraid state, time, I/O, and policy. Familiar-looking
   cleverness is not simple.
3. **Parse, don't validate** — at the boundary, raw input becomes a type that
   proves the check. Interior code never re-checks a string.
4. **Functional core, imperative shell** — decisions are data-in/data-out.
   Clock, disk, network, and UI stay in the shell.
5. **Seams pay rent** — inject clock, I/O, or a real second implementation.
   Do not add a one-impl interface.
6. **Chesterton's Fence** — if you cannot say why a branch exists, leave it
   and flag it. Never delete to tidy.
7. **Idiom of this repo** — match neighboring Mitsuro patterns. Do not import
   a foreign style.

A slice that cannot name which rule it preserves does not ship.

## Hard reject

See [reject-list.md](reject-list.md). Short form:

- Dead-code, unused-import, unused-param, or "git remembers" deletion
- Public contract, compatibility reader, migration, schema, or recovery-state change
- UI appearance, interaction, or copy change
- Test deletion, weakening, or rewrite to make a refactor pass
- Cross-crate logic moves that violate `mitsuro-cli` / `mitsuro-core` / `mitsuro-server`
- Broad rewrite, drive-by rename, comment churn
- Error-context or failure-mode removal
- Speculative abstraction, one-impl factory, config for fewer than three values
- Performance change without a named bottleneck

## Fleet

Spawn these Cursor agents. Do not use a generic explore/build stand-in when
these exist. Launch read-only agents in parallel. Never let two writers touch
the same sector.

| Agent | Role | Writes |
|---|---|---|
| `quality-scout` | Find shallow modules, complected code, missed parses, unpaid seams | No |
| `quality-modularize` | Propose or apply one deepening / idiom extraction | Only in `apply` |
| `quality-perf` | Named hot-path change with the same workload before/after | Only in `apply` |
| `quality-simplifier` | Polish expression of an already-approved slice | Only in `apply` |
| `quality-critic` | Veto quality, design, state, or contract loss | No |

The parent agent is the orchestrator. It picks the sector, scores candidates,
enforces the reject list, runs validation, and updates the ledger.

## One tick

Copy this checklist and keep it in the reply.

```
Tick:
- [ ] Mode: propose-only | apply
- [ ] Sector: <one crate/module path>
- [ ] Scout report received
- [ ] Candidates scored
- [ ] Reject-list filter applied
- [ ] One slice chosen (or none)
- [ ] Critic passed or slice dropped
- [ ] Validation passed or slice reverted
- [ ] Ledger updated
```

### 1. Pick one sector

One directory, one crate boundary. Prefer 1k–8k LOC. Default first sector is
`crates/mitsuro-core/src/plan`. Do not activate the monorepo. Run
`scripts/sectors.sh` if you need a size map.

If the user named a path, use that path only.

### 2. Scout

Spawn `quality-scout` with:

```
SECTOR: <path>
MODE: propose-only
Read .cursor/skills/quality-improve/SKILL.md and reject-list.md.
Return a Scout Report only. Do not edit files.
```

### 3. Score

Keep a candidate only if it is one of:

- **Deepen** — deletion test says complexity would move to callers if removed
- **Unbraid** — state, time, I/O, or policy are mixed and can be split
- **Parse once** — raw data is re-checked inside instead of a boundary type
- **Idiom** — two or three call sites already want the crate's existing helper
- **Hot path** — named bottleneck on a render, stream, or tool path

Score each remaining candidate:

`modularity + idiom + evidence - blast_radius`

Drop anything the reject list hits. Drop anything without a file:line.

### 4. Propose or apply one slice

Propose-only: write the slice as a patch description. Do not edit product code.

Apply: spawn exactly one of `quality-modularize` or `quality-perf` for that
slice. Then spawn `quality-simplifier` on the same diff only. No extra cleanup.

### 5. Critic

Spawn `quality-critic` on the proposal or the diff. Any P0/P1, any reject-list
hit, or any "tests must change" finding → drop or revert. Do not fix-forward
into a bigger rewrite.

### 6. Validate

Propose-only: no compile required. Check that cited paths exist and the
proposal does not require test edits.

Apply, after the write:

```bash
# Rust sector
cargo fmt --all
cargo check -p <crate>
# plus the narrowest test for the touched module

# TS / mobile sector
cd apps/mobile && npx tsc --noEmit
```

Full workspace gates are for the commit that lands an apply slice, not for
propose-only.

If validation fails → revert the slice, ledger it as `reverted`, stop the tick.

### 7. Ledger

Append one row to [ledger.md](ledger.md). Never re-litigate a `kept`,
`rejected`, or `reverted` slice unless the user asks.

### 8. Stop

Stop the tick after one slice, an empty candidate list, a critic reject, or a
validation revert. Do not start the next sector unless the user asked for a
fleet run of N ticks.

A fleet run is `N` sequential ticks, still one sector and one slice each,
still propose-only unless apply was granted.

## Readiness to apply

Apply mode is allowed only when all of these are true:

- [ ] Skill, reject list, and five agents are in the tree
- [ ] At least one propose-only tick produced a critic-reviewed candidate list
- [ ] That tick changed no product code
- [ ] The user explicitly said apply
- [ ] The chosen slice is on the latest propose-only list

If any box is open, stay propose-only and say why.

## Mitsuro invariants the critic must keep

- Shared logic stays in `mitsuro-core`. CLI and server stay thin.
- Tool path policy stays on `ToolContext`.
- Provider quirks stay in the AI transform layer.
- Plan lifecycle, permission mode, and turn budgets stay canonical.
- Storage migrations stay forward-only. Recovery state stays out of history.
- Mobile/web: no render-time network, no whole-transcript re-render, no visual change.
- Performance claims need the same build and the same workload.

## Fire

```
propose-only, one sector:
  Run quality-improve propose-only on crates/mitsuro-core/src/plan

apply one listed slice:
  Apply quality-improve slice <id> from the latest ledger

fleet of N propose-only ticks:
  Run quality-improve propose-only for 3 ticks starting at crates/mitsuro-core/src/plan
```
