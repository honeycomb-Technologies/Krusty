# Krusty iOS shell

This folder is the native shell lane for the GPUI mobile app.

Current split:

- GPUI/Rust owns in-app chat UI and state (`crates/krusty-mobile`, `krusty-client-state`).
- Swift owns OS integrations: Keychain, deep links, attachment pickers, native multiline composer, and WKWebView browser/terminal surfaces.
- `KrustyMobileShell` is a Swift package so it can be added to an Xcode iOS app target without mixing shell code into Rust crates.
- `KrustyMobileApp` is a minimal checked-in Xcode iOS app target that links the shell package and exercises the browser, terminal, and composer bridge surfaces.

## First iPhone path

1. Run the Krusty server on the MacBook:
   ```bash
   cargo run -p krusty -- serve
   ```
2. Expose the server to the phone/simulator with localhost, LAN IP, or the existing remote-access route.
3. Build/run the Rust GPUI preview locally:
   ```bash
   KRUSTY_MOBILE_SERVER=http://127.0.0.1:3000 cargo run -p krusty-mobile
   ```
4. Open `apps/ios/KrustyMobileApp/KrustyMobileApp.xcodeproj` in Xcode.
5. Run the `KrustyMobileApp` scheme on simulator/device. The app now hosts a `GpuiHostView`; if the Rust static library is not linked yet, it falls back to shell-only mode and reports that in the status label.

The Swift package intentionally does not contain litter-ish code. Local Linux runtime work stays in `experiments/litter-ish-spike` until GPL/App Store and host↔guest RPC gates are resolved.

## GPUI embed path

Krusty patches crates.io `gpui = 0.2.2` from `vendor/gpui` so `gpui-component = 0.5.1` keeps compiling while Krusty adds an internal iOS backend. `crates/krusty-mobile` can be built as an iOS `staticlib` and exposes the following C ABI entrypoint for Swift-owned host views:

```c
void krusty_mobile_start_with_host_view(void *ui_view);
```

`KrustyMobileShell.GpuiRuntimeBridge` intentionally resolves Rust symbols with `dlsym`, so the checked-in iOS app still builds before the Rust library is linked. Build the simulator archive on a Mac with:

```bash
scripts/mobile-ios-rust-staticlib.sh aarch64-apple-ios-sim
```

When linking the Rust static library, make sure Xcode force-loads it (for example with `-force_load path/to/libkrusty_mobile.a`) so the dlsym-only symbols are not stripped by the static linker.

## Mac/iOS validation

On a Mac with Xcode and Rust iOS targets installed:

```bash
scripts/mobile-ios-mac-smoke.sh
```

This builds:

- `KrustyMobileShell` for generic iOS.
- `KrustyMobileApp` for generic iOS Simulator.
- `KrustyMobileApp` for generic iOS without signing.
- `krusty-client` and `krusty-client-state` for `aarch64-apple-ios-sim`.

Optional hard gate:

```bash
MOBILE_IOS_CHECK_GPUI=1 scripts/mobile-ios-mac-smoke.sh
```

This now checks the vendored GPUI iOS backend and `krusty-mobile` against `aarch64-apple-ios-sim`. It must run on macOS because Apple targets require Xcode's `xcrun`/SDK toolchain.

## MacBook over Tailscale

From this Linux dev box, check whether the MacBook is reachable before trying remote builds:

```bash
KRUSTY_MACBOOK_TAILSCALE_HOST=haleys-macbook-air scripts/mobile-macbook-ssh-check.sh
KRUSTY_MACBOOK_TAILSCALE_HOST=haleys-macbook-air KRUSTY_MACBOOK_SSH_USER=<mac-user> scripts/mobile-macbook-ssh-check.sh
KRUSTY_MACBOOK_TAILSCALE_HOST=haleys-macbook-air KRUSTY_MACBOOK_SSH_USER=<mac-user> KRUSTY_MACBOOK_SSH_KEY=<private-key> scripts/mobile-macbook-ssh-check.sh
KRUSTY_MAC_HOST=haleys-macbook-air KRUSTY_MAC_USER=<mac-user> KRUSTY_MAC_SSH_KEY=<private-key> scripts/mobile-macbook-ssh-check.sh
```

If the first command cannot ping, the MacBook is likely asleep/offline or Tailscale ACLs/SSH are not open. If ping works but SSH fails, enable macOS **System Settings → General → Sharing → Remote Login** and make sure this machine's SSH key is authorized. When SSH works, the script also prints whether `xcodebuild`, `swift`, `cargo`, and installed Rust iOS targets are present.
