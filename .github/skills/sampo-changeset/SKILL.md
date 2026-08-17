---
name: sampo-changeset
description: Create or update Sampo changesets for Mitsuro version bumps and changelogs.
---

Mitsuro uses [Sampo](https://github.com/bruits/sampo) for changelogs, SemVer, and
`v{version}` tags. Pending changesets live in `.sampo/changesets/`.

## When to add a changeset

Add one for any user-visible product change that should appear in a GitHub
Release: CLI, Honey/server, Hive, mobile, or desktop behavior. Skip
internal-only refactors, test-only work, and docs that do not change product
behavior.

## Create a changeset non-interactively

```sh
sampo add -p cargo/mitsuro -b patch -t Added -m "Short user-facing description."
```

- `-b` is `major`, `minor`, or `patch`.
- `-t` is one of `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.
- Repeat `-p` to target more packages. Use `cargo/mitsuro-hive` for Hive-only
  daemon/protocol work. `cargo/mitsuro` is in a fixed group with the other 0.9.x
  product crates, so one bump covers CLI, core, server, and desktop.

You can also write `.sampo/changesets/<slug>.md` directly. See
`.sampo/changeset.md.example`.

## Do not

- Do not retag `v0.9.22` or any existing protected tag.
- Do not `cargo publish` product crates. They are `publish = false`.
- Do not run `sampo publish` from a laptop unless you intend to create tags.
  Version (Sampo) tags after the version PR merges; Release binaries then
  uploads GitHub Release assets. A tag without those assets is not a release.
  See `docs/operations/release.md`.
