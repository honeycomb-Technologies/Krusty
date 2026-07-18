# Mako Specs

Product, IA, roadmap, and implementation notes for the Mako surface live here.

## Key Docs

- [MAKO_BACKEND_ARCHITECTURE_V2.md](./MAKO_BACKEND_ARCHITECTURE_V2.md)
- [COMPETITOR_SOURCE_REVIEW_2026-07-17.md](./COMPETITOR_SOURCE_REVIEW_2026-07-17.md)
- [KRUSTY_MAKO_PRODUCT_MODEL_V3.md](./KRUSTY_MAKO_PRODUCT_MODEL_V3.md)
- [KRUSTY_MAKO_REPLACEMENT_IA.md](./KRUSTY_MAKO_REPLACEMENT_IA.md)
- [KRUSTY_MAKO_SCREEN_MAP_V2.md](./KRUSTY_MAKO_SCREEN_MAP_V2.md)
- [KRUSTY_MAKO_IMPLEMENTATION_PLAN_V2.md](./KRUSTY_MAKO_IMPLEMENTATION_PLAN_V2.md)

## Durable data privacy

Schema migration 43 replaces legacy raw Mako controller-event payloads and
execution error/outcome copies with allow-listed summaries. Migration 44 is the
crash-safe physical-cleanup checkpoint: it is recorded only after SQLite secure
deletion, WAL truncation, `VACUUM`, and a final checkpoint have completed. If a
process stops after logical redaction but before physical cleanup, the database
remains at schema 43 and the next opener repeats cleanup before advancing to 44.

That migration cannot rewrite copies outside the configured live database.
Before upgrading a production host, expire or securely rotate old database
backups, volume snapshots, copied `-wal` files, and support bundles according to
their retention policy. Treat every pre-v43 backup as potentially containing
model reasoning, provider signatures, tool arguments/output, web content, and
raw execution errors. Restoring one requires running the current migration
before it is exposed to clients or copied into a new backup set.

## Supporting Specs

- [KRUSTY_MAKO_ATTENTION_SPEC_V1.md](./KRUSTY_MAKO_ATTENTION_SPEC_V1.md)
- [KRUSTY_MAKO_DRAWER_TRANSITION_V1.md](./KRUSTY_MAKO_DRAWER_TRANSITION_V1.md)
- [KRUSTY_MAKO_HYBRID_DESIGN.md](./KRUSTY_MAKO_HYBRID_DESIGN.md)
- [KRUSTY_MAKO_PHASE1_IMPLEMENTATION_SPEC.md](./KRUSTY_MAKO_PHASE1_IMPLEMENTATION_SPEC.md)
- [KRUSTY_MAKO_ROADMAP.md](./KRUSTY_MAKO_ROADMAP.md)
- [KRUSTY_MAKO_SCHEDULE_SPEC_V1.md](./KRUSTY_MAKO_SCHEDULE_SPEC_V1.md)
- [KRUSTY_MAKO_TAB_SPEC_V1.md](./KRUSTY_MAKO_TAB_SPEC_V1.md)
