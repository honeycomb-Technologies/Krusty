# GPUI Mobile implementation checkpoint

Krusty Mobile is now chat-first and Mako is on hold.

## Implemented foundation

- `crates/krusty-client`: async Rust HTTP/SSE client for health, models, sessions, session state, server access/status, chat streaming, and tool approvals.
- `crates/krusty-client-state`: headless chat state machine with transcript nodes, stored session reload, live partial recovery, streaming deltas, plan updates, approvals, accordion controls, attachments, and shell actions.
- `crates/krusty-mobile-ui`: lightweight GPUI primitives for desktop-styled mobile transcript rendering.
- `crates/krusty-mobile`: GPUI phone-sized preview app with normal chat, folder/research/paper/terminal/browser top surfaces, desktop-square styling, and the crab accordion controls.
- `apps/ios/KrustyMobileShell`: Swift package for Keychain, deep links, native composer, attachment picker, and WKWebView browser/terminal bridges.
- `apps/ios/KrustyMobileApp`: minimal checked-in iOS app target that links the shell package and exercises browser/terminal/composer bridge surfaces while GPUI iOS is gated.
- `experiments/litter-ish-spike`: isolated local-runtime spike plan.
- `scripts/mobile-runtime-smoke.sh`: local backend/mobile launch smoke test.
- `scripts/mobile-macbook-ssh-check.sh`: Tailscale/SSH reachability check for MacBook builds.

## Run the GPUI mobile preview

```bash
cargo run -p krusty -- serve
KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 cargo run -p krusty-mobile
KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 KRUSTY_MOBILE_SESSION_ID=<session-id> cargo run -p krusty-mobile
```

## Runtime smoke test

```bash
KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 scripts/mobile-runtime-smoke.sh
KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 scripts/mobile-runtime-smoke.sh --launch
KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 scripts/mobile-runtime-smoke.sh --chat --launch
```

The first command checks health, models, credentials, server access/status, sessions, session creation, and session state without spending model tokens. `--launch` briefly compiles/runs the GPUI preview and treats a still-open window timeout as success. `--chat` also makes a real model-backed chat request.

## MacBook/iPhone path

```bash
scripts/mobile-macbook-ssh-check.sh
KRUSTY_MACBOOK_SSH_USER=<mac-user> scripts/mobile-macbook-ssh-check.sh
KRUSTY_MACBOOK_SSH_USER=<mac-user> KRUSTY_MACBOOK_SSH_KEY=<private-key> scripts/mobile-macbook-ssh-check.sh
KRUSTY_MAC_HOST=haleys-macbook-air KRUSTY_MAC_USER=<mac-user> KRUSTY_MAC_SSH_KEY=<private-key> scripts/mobile-macbook-ssh-check.sh
```

If the MacBook is reachable over Tailscale and SSH is enabled, the second command confirms whether this Linux workstation can drive Mac-side `xcodebuild`/device work remotely.

## Mac/iOS smoke test

Run this on the Mac checkout:

```bash
scripts/mobile-ios-mac-smoke.sh
```

It validates the Swift shell package, the checked-in iOS app target for simulator/device compilation, and the Rust `krusty-client`/`krusty-client-state` crates for `aarch64-apple-ios-sim`.

The optional GPUI hard gate is intentionally separate:

```bash
MOBILE_IOS_CHECK_GPUI=1 scripts/mobile-ios-mac-smoke.sh
```

Current result: `krusty-mobile` fails on `aarch64-apple-ios-sim` inside upstream `gpui` because `current_platform` and screen-capture platform types are only implemented for macOS/Linux/Windows. The client/state and Swift shell are iOS-ready; the next GPUI step is an upstream/adapter iOS platform layer before embedding can proceed.

## Product boundaries

- GPUI/Rust owns in-app UI and state.
- Swift owns iOS integrations.
- WKWebView owns terminal/browser bridges for v1.
- litter-ish stays separate until boot, RPC, performance, and license gates pass.
- Mako surfaces remain deferred for redesign.
