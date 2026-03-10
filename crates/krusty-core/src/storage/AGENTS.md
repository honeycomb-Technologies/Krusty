# AGENTS Guide: /crates/krusty-core/src/storage

## Purpose
SQLite persistence for sessions, plans, credentials, push observability, and compact runtime trace replay data.

## Guardrails
- Migration safety first: schema changes must be forward-only and tested.
- Keep read/write behavior explicit and transaction-aware.
- Keep interrupted-turn recovery state separate from canonical conversation history.
- Keep runtime traces compact and structured; persist summarized diagnostic payloads rather than raw stream dumps unless exact replay fidelity is required by design.
- Linked-session persistence must preserve parent ownership metadata so pinch/continuation flows do not escape multi-tenant boundaries.
- For push reliability changes, keep `database.rs`, `push_subscriptions.rs`, and `push_delivery_attempts.rs` aligned.
- Never log sensitive credentials.

## Validation
- `cargo check -p krusty-core`
- run targeted storage migration tests for schema changes.
