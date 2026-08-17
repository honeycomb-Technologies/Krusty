# Sampo

Mitsuro uses [Sampo](https://github.com/bruits/sampo) for changesets, SemVer
bumps, changelogs, and git tags.

- Add a changeset: `sampo add -p cargo/mitsuro -b patch -t Added -m "..."`
- Prepare versions: `sampo release` (or merge the Release PR CI opens on `main`)
- Publish tags: `sampo publish` (CI does this after the Release PR merges)

The product tag is `v{version}` on `cargo/mitsuro`. That tag starts the existing
binary release workflow. Crates are not published to crates.io.

See `CONTRIBUTING.md` and `AGENTS.md` for the full workflow.
