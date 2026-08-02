# Contributing to Mitsuro

Thanks for helping improve Mitsuro.

## Before you start

- Search existing issues before opening a new one.
- Keep each change focused on one problem.
- Do not include credentials, private logs, customer data, local paths, research
  notes, staging handoffs, or generated build output.
- Read [AGENTS.md](AGENTS.md) before changing the codebase. It contains the
  repository's engineering and release rules.

## Local setup

Build and test the Rust workspace:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```

Validate the shared client:

```bash
cd apps/mobile
bun install --frozen-lockfile
npx tsc --noEmit
npx expo export --platform web
```

Run the local server from the repository root:

```bash
cargo run -p mitsuro
```

Run the Expo client in another terminal:

```bash
cd apps/mobile
npx expo start
```

## Pull requests

A good pull request:

- explains the user-visible problem and outcome;
- lists the checks that were run;
- includes screenshots or recordings for visible UI changes;
- updates documentation when behavior or setup changes;
- preserves compatibility unless a migration is part of the change;
- avoids unrelated formatting or cleanup.

Use Mitsuro for the harness and Hive for durable autonomous work in code,
documentation, tests, packages, routes, storage, and release artifacts. Keep
any prior-identity reader isolated to a tested compatibility boundary.

## Reporting bugs

Please include:

- Mitsuro version or commit;
- operating system and app surface;
- steps that reproduce the problem;
- expected and actual behavior;
- relevant logs with credentials, tokens, private paths, and personal data
  removed.

Security issues should follow [SECURITY.md](SECURITY.md), not a public issue.
