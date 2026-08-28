# Quality Improve Examples

## Fire propose-only

User: `Run quality-improve propose-only on crates/mitsuro-core/src/plan`

Orchestrator:

1. Reads this skill and the reject list.
2. Spawns `quality-scout` on that path only.
3. Scores and filters.
4. Spawns `quality-critic` on the candidate list.
5. Appends a `proposed` row to `ledger.md`.
6. Does not edit Rust, TS, or tests.

## Fire apply

User: `Apply quality-improve slice plan-2 from the latest ledger`

Orchestrator:

1. Confirms readiness and that `plan-2` is on the latest propose-only list.
2. Spawns `quality-modularize` with that slice only.
3. Spawns `quality-simplifier` on the resulting diff.
4. Spawns `quality-critic` on the diff.
5. Runs `cargo fmt --all` and `cargo check -p mitsuro-core` plus the plan tests.
6. Keeps or reverts. Updates the ledger.

## Good slice

Extract a repeated plan-status resolution that UI and server both re-derive
into the existing core helper. Interface shrinks. Callers pass data, get a
typed status. Tests stay as-is.

## Bad slice

Delete an unused-looking `legacy_*` reader, rename a public route field, or
flatten a recovery branch because "nothing calls it from this crate".
