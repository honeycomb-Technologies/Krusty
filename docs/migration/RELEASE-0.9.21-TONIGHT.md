# Release 0.9.21 — Tonight cut (Mitsuro board-wide)

**Goal:** Ship one version (`0.9.21`) across CLI/server/mobile packaging, archive
legacy TUI so only **tui_v2** is the product terminal, merge staging to `main`,
tag `v0.9.21`, and launch a fresh Honey install from that tip.

## Non-negotiables

1. **Apple pipeline stays `io.krusty.mobile`** (APNS, TestFlight continuity).
   Display name remains Mitsuro. Deep-link schemes keep both `mitsuro` and `krusty`.
2. **CLI compatibility shims stay:** `krusty` → `mitsuro`, `krusty-mako` → `mitsuro-hive`.
3. **No silent main merge without validation green.**
4. **Honey production restart only from the release tip/tag.**

## Phases

| ID | Work | Done when |
|----|------|-----------|
| R0 | Archive `archive/tui-v1-20260802` on origin | branch pushed |
| R1 | Remove product v1 TUI; keep shared support as `tui_support` | default entry only tui_v2; no handlers/blocks/popups |
| R2 | Version **0.9.21** on Rust product crates + mobile app/package | all match |
| R3 | Pin iOS/Android store IDs to `io.krusty.mobile` / groups | matches Honey APNS |
| R4 | README terminal docs accurate | documents tui_v2 |
| R5 | `cargo check/test/clippy/fmt` | green |
| R6 | Merge staging → main, tag `v0.9.21`, push | GitHub has tag |
| R7 | Build release binaries on Honey, install, restart serve+hive | health OK |

## After release

CLI-as-client-of-server migration starts from this tip (not in this cut).

