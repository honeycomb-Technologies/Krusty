# Quality Improve Reject List

If a candidate matches any row, drop it. Flag it in the scout report if it
looked tempting. Do not "just this once".

## Deletion

- Unused functions, methods, types, or modules that look unreferenced
- Unused parameters, imports, or bindings as the purpose of the slice
- Unreachable branches you cannot prove are unreachable from every caller
- Commented-out code removal as the purpose of the slice
- "Git remembers" or "YAGNI" used to drop a path
- Compatibility readers, shims, or deprecated aliases
- Feature flags, session recovery, draft recovery, or interrupted-turn state

Absence of a static caller is not proof of death. Hive, ACP, mobile, desktop,
TUI, and server all call core.

## Contracts and state

- Public API shrink or rename without an explicit user request
- Schema, migration, or SQLite table changes
- Session, plan, credential, or preference persistence shape changes
- Recovery, draft, or compaction checkpoint format changes
- Auth, permission mode, or turn-budget semantics
- Product language swaps that leak internal names into UI/API

## Design and product

- UI appearance, spacing, motion, copy, or interaction changes
- New user-visible mode, setting, or surface
- Rewriting a module "while we are here"
- Drive-by renames, comment-only diffs, or doc churn
- Import-order or formatting-only slices (let `cargo fmt` / the formatter own that)

## Tests

- Deleting tests
- Weakening assertions
- Rewriting a test so a refactor passes
- Adding tests that change the contract instead of locking current behavior

A slice that needs a test edit to stay green changed behavior. Revert.

## Architecture

- Moving shared logic into `mitsuro-cli` or `mitsuro-server`
- Duplicating `ToolContext` path policy, plan lifecycle, or permission resolution
- New standalone validators, catalogs, or shadow stores
- One-implementation interfaces, factories, or strategy trees
- Config files or builders for fewer than three real values
- Event buses, plugin seams, or middleware for fewer than three consumers
- Cross-crate moves that ignore `packages/*`, `apps/*`, `crates/*` boundaries

## Performance

- Micro-opts with no named bottleneck
- Caching without a measured or clearly hot path
- "Faster" that is harder to read and unmeasured
- Changing a stream, queue, or render path without naming the workload

## Expression

- Nested ternaries, dense one-liners, or golfed iterators that need a pause
- Removing error context, `warnings`, or explicit failure modes
- Inlining a helper that gave a domain concept its name
- Merging unrelated functions to cut file count
- Expanding a deep module into many shallow files that push work to callers
