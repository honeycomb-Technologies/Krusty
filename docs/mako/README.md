# Mako Engineering Documentation

Current Mako documentation is intentionally limited to implemented runtime and
operational behavior:

- [Backend Architecture](./MAKO_BACKEND_ARCHITECTURE_V2.md)
- [Mako Autonomous Mode](../interfaces/mako-autonomous-mode.md)
- [Build and Deployment](../operations/build-and-deploy.md)

Historical product models, competitive reviews, roadmaps, screen maps, and
implementation plans are preserved in the
[Mako Product History](../archive/mako-product-history/README.md). They are not
the current product contract.

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
