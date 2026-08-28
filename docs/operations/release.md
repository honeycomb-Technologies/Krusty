# Cutting a Mitsuro release

One operator path. A cut is **released** only when the GitHub Release has
the linux archive. A git tag without assets is not a release. `systemctl
is-active` is not Honey.

Sampo is not a faster GitHub Release. It only turns changesets into version
bumps, changelogs, and `v{version}` tags. **Release binaries** is what
`install.sh` and Honey consume.

## The path

1. Integrate on the current `codex/release-staging-YYYYMMDD` branch.
2. Promote staging → `main` (`sh scripts/promote-staging.sh`, then merge).
3. User-visible work already has a changeset (`.sampo/changesets/`).
4. **Version (Sampo)** on `main` opens a version PR, or tags if that PR just
   merged. It restores `Cargo.lock` third-party pins and syncs Expo.
5. **Release binaries** builds CLI, Hive, and desktop and uploads the GitHub
   Release. Sampo publish and CI on `main` start this automatically. If it
   fails, fix the cause and re-run **Release binaries** with the same tag.
6. Honey: `sh scripts/honey-upgrade.sh v0.9.23` only after
   `sh scripts/release-status.sh` says the linux archive is present.
7. TestFlight is separate. It starts when a GitHub Release is published, or
   from **Mobile → TestFlight** `workflow_dispatch`. It is not a side effect
   of bumping `app.json`.

```bash
sh scripts/promote-staging.sh
sh scripts/release-status.sh
sh scripts/honey-upgrade.sh v0.9.23
```

## Retry

| Failure | Correction |
|---|---|
| Version PR missing (Actions cannot open PRs) | Enable Actions pull-request creation, or open `release/sampo` → `main` by hand |
| `Cargo.lock` jumped third-party crates | `sh scripts/refresh-workspace-lock-versions.sh origin/main` on the version branch |
| Expo version drifted | `sh scripts/sync-product-version.sh` |
| Tag exists, no linux archive | Actions → **Release binaries** → Run workflow → that `v*` tag |
| Binary job failed | Fix the job, re-run the same workflow; existing assets are not overwritten |
| Honey `404` | Do not restart services. Wait for the archive, then `honey-upgrade.sh` |
| Honey `is-active` but `/health` fails | `journalctl --user -u mitsuro-serve.service -u mitsuro-hive.service -n 120` |
| Browser `API 503` / missing Atlas runtime | The linux archive already has `agent-browser`. On the Honey host run `sh scripts/honey-atlas-repair.sh`. Do not only restart units. |

Do not retag a protected `v*`. Do not `cargo publish`. Do not restart
`mitsuro-hive.socket` by itself (`RemoveOnStop=true` deletes the IPC socket).

## What each workflow is

| Workflow | Does | Does not |
|---|---|---|
| **Version (Sampo)** | Bump versions, changelogs, tags | Upload Honey-installable archives |
| **Release binaries** | Build and publish GitHub Release assets | Change SemVer |
| **CI** | Quality gates; start a missing binary cut on `main` | Deploy Honey |
| **Mobile → TestFlight** | EAS iOS when a Release is published, or manual dispatch | Prove Honey is live |
