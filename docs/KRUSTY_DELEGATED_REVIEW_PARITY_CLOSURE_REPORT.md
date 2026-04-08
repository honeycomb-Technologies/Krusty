## Summary

The delegated review parity pass is complete.

Krusty now follows the OpenCode-style external contract more closely:

- delegated child artifacts remain runtime/data contracts
- the parent-owned `human_review` is the canonical final `explore` answer
- completed delegated cards no longer compete with the final review
- delegated run identity survives stored/reloaded session messages

## Code Seams Updated

- `crates/krusty-core/src/tools/implementations/explore.rs`
- `crates/krusty-core/src/agent/orchestrator.rs`
- `apps/pwa/app/src/lib/stores/session.ts`
- `apps/pwa/app/src/lib/components/chat/Message.svelte`
- `apps/pwa/app/src/lib/components/chat/ToolWidget.svelte`
- `apps/pwa/app/src/lib/components/chat/DelegatedToolWidget.svelte`

## Validation

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --all --check`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

## Result

Delegated `explore` runs now present as:

- one contained delegated run card for progress/state
- one parent-written review for the actual answer

Telemetry remains available in the delegated card/details and persisted delegated run state, but it is no longer the primary chat response.
