# Mitsuro and Hive identity migration

Status: implementation on `codex/release-staging-20260801`.

## Canonical names

| Surface | Canonical identity |
| --- | --- |
| Product and terminal command | Mitsuro / `mitsuro` |
| Shared runtime crate | `mitsuro-core` (`mitsuro_core`) |
| HTTP service crate and binary | `mitsuro-server` / `mitsuro serve` |
| Autonomous system | Hive |
| Hive daemon and protocol crates | `mitsuro-hive`, `mitsuro-hive-protocol` |
| JavaScript packages | `@mitsuro/*` |
| HTTP routes | `/api/hive/*` |
| Stored session kind | `hive` |
| Data and configuration roots | `~/.mitsuro`, `~/.config/mitsuro` |
| Primary database | `mitsuro.db` |
| Environment prefix | `MITSURO_`; Hive-specific values use `MITSURO_HIVE_` |
| User services | `mitsuro-serve.service`, `mitsuro-hive.service`, `mitsuro-hive.socket` |
| Deep links | `mitsuro://` |
| Mobile and desktop identifiers | `io.mitsuro.mobile`, `group.io.mitsuro.mobile`, `io.mitsuro.desktop` |

## Compatibility boundary

This branch makes the canonical identity primary and isolates prior identifiers at
explicit migration boundaries. Compatibility readers and aliases exist only to
upgrade installed clients, stored data, plugins, HTTP callers, environment
configuration, and service installations. They must not appear as current product
copy or leak back into newly written state.

The bridge release must:

1. Prefer canonical paths, environment variables, routes, serialized values, and
   package names.
2. Read the prior identity only through a dedicated compatibility helper, migrate
   it once, and write only canonical state afterward.
3. Accept both HTTP and IPC generations during a mixed-version window without
   allowing two schedulers to own one database.
4. Preserve opaque historical IDs and signed plugin payloads; never recompute them.
5. Provide canonical binaries and service units before any installed alias is
   retired.

## Cutover boundaries

Repository implementation, local builds, migration fixtures, generated web assets,
and package/service definitions are in scope here. App Store Connect, Apple
Developer, Google Play, EAS project/credential changes, publishing, production
service restarts, and live durable-data migration remain separate approved actions.

The code cutover is accepted only after a fresh-install fixture and a pre-migration
fixture both pass database integrity and row-parity checks, old and new client/API
compatibility tests pass, the generated web bundle contains only current product
copy, and all required Rust and client validation gates pass.

## Platform state cutover

- Linux shell-installer upgrades must use the current canonical installer. It
  stops the
  supervised old generation, records a non-mutating database/WAL digest
  manifest, invokes `mitsuro migrate-identity --confirm-offline`, verifies that
  the source authority did not change, and only then starts canonical units.
- macOS shell installation stages and verifies the release but fails closed on
  automatic cutover because it lacks the Linux procfs proof. The operator must
  stop both generations, run the exact staged physical migration command the
  installer prints, and rerun the installer without starting Mitsuro between
  those steps.
- Windows direct installation publishes `mitsuro.exe` and its transition
  command copy only. When the prior state root exists, the operator must stop
  every old and canonical server/Hive process and run
  `mitsuro migrate-identity --confirm-offline` before normal startup.
- Homebrew and AUR install binaries or units only; they require the same manual
  offline migration before first startup or service enablement when previous
  state exists.
- Desktop web-data migration is automatic only for the receipt-backed Linux XDG
  and Windows LocalAppData roots. Wry does not expose authoritative WKWebView
  storage on macOS, so the shell does not copy a guessed Application Support
  directory. Preserve old macOS desktop data until a signed build passes manual
  cookies/localStorage, connection, authentication, and preference continuity
  checks; re-authentication is not proof of migration.
- State cutover acceptance requires the regular, at-most-16-KiB
  `.identity-migration-v2` file with exactly five ordered LF-terminated fields,
  the exact preserved source root, bounded numeric fields, lowercase SHA-256
  logical-SQLite and durable-tree fingerprints, and a consistent WAL stat
  tuple. A structurally plausible v1 or partial receipt is not accepted.
- The migration binary, not the installer shell, owns the held SQLite writer
  fence and online backup. The previous root remains rollback authority; a
  failed canonical root is quarantined rather than merged back or deleted.
- After a successful cutover, the previous root, physical binaries, and desktop
  app are recovery-only. No continuous lock prevents a same-user direct launch;
  never run the previous generation except through a coordinated rollback that
  first proves every canonical process quiescent and selects the previous
  release and state together.

## Local cleanup boundary

Old worktrees, branches, repositories, generated build trees, installed releases,
and runtime state are inventoried separately. Nothing with unique commits, dirty
work, an open database, a running process, or unverified recovery is deleted during
the source migration. Clean redundant candidates may be removed only after their
recovery proof is recorded and the user approves the destructive cleanup set.
