# litter-ish local-runtime spike

Purpose: evaluate whether `dnakov/litter-ish` can eventually let Krusty run locally on iPhone without making it the v1 mobile foundation.

## Current decision

- Do **not** link or vendor litter-ish into MIT Krusty crates yet.
- Keep this as a separate Xcode target/submodule experiment because litter-ish is iSH/GPL-derived and App Store/legal obligations need explicit review.
- The production mobile app remains chat-first: Rust client/state + GPUI UI + Swift OS shell + WKWebView terminal/browser bridge.

## Spike gates

1. Boot the ARM64 Alpine fakefs on iOS hardware/simulator target.
2. Run small CLI workloads: `sh`, `apk`, `python`, `node`, `cargo --version` if available.
3. Verify networking and security-scoped filesystem mounts.
4. Build a minimal host↔guest RPC bridge for command execution and streamed stdout/stderr.
5. Measure battery, startup time, memory, and background behavior.
6. Complete GPL/App Store compliance review before any product integration.

## Non-goals for v1

- No Servo browser engine.
- No local Krusty runtime in the main app binary.
- No GPL source copied into `crates/*`.

## Candidate checkout

Use an external checkout or submodule outside `crates/*`, for example:

```bash
mkdir -p third_party/spikes
git clone https://github.com/dnakov/litter-ish third_party/spikes/litter-ish
```

Keep any glue code in an isolated app target until the gates above pass.
