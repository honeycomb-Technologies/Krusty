# Desktop App (GPUI)

Mitsuro's canonical desktop is the native Rust client in `apps/desktop/gpui`.
It uses GPUI for the application/window/rendering runtime and GPUI Component for
inputs, icons, and shared controls. The ordinary application shell is not HTML,
React, Electron, or a Tauri webview.

## Product architecture

The desktop renders one normalized `ProductBackend` at a time:

- `mitsuro-http` connects to the Mitsuro server through HTTP and SSE.
- `codex-stdio` owns a managed `codex app-server --stdio` child.
- fixture mode is explicit and restricted to deterministic tests and review.

Settings can switch between the two live transports without restarting GPUI. Session
identity, model selection, pins, preferences, and asynchronous results are namespaced
by backend so stale work from the previous transport cannot mutate the new view.
Feature controls follow the active backend's real capability contract. Missing product
APIs render loading, empty, unsupported, or error states; a production connection never
falls back to fixture records.

The detailed capability matrix and validation commands live in
[`gpui-desktop.md`](gpui-desktop.md).

## Native and web boundaries

GPUI owns the window, navigation, transcript, composer, approvals, terminal, files,
settings, Work, Scheduled, and other product surfaces. WebKitGTK is linked for the
sandboxed MCP Apps extension: validated MCP HTML is rendered in an ephemeral,
permission-denying offscreen runtime and presented as GPUI image frames. The default
Atlas surface opens real pages in the system browser and never fabricates page content.

## Development

Install the Rust toolchain plus GTK 3, WebKitGTK 4.1, and xkbcommon development
packages, then run:

```bash
cargo run -p mitsuro-gpui-desktop
```

Useful read-only live modes are:

```bash
MITSURO_BACKEND=mitsuro-http MITSURO_NO_LIVE_TURN=1 \
  cargo run -p mitsuro-gpui-desktop

MITSURO_BACKEND=codex-stdio MITSURO_NO_LIVE_TURN=1 \
  cargo run -p mitsuro-gpui-desktop
```

`CODEX_BIN` can point at a specific Codex executable. The selected transport and
privacy-safe local preferences are stored in `~/.mitsuro/gpui-desktop-state.json`;
credentials and server tokens are not stored there.

## Linux packaging

Tagged Linux releases build the optimized `mitsuro-gpui-desktop` Cargo binary and call:

```bash
scripts/package-gpui-desktop.sh \
  target/release/mitsuro-gpui-desktop \
  artifacts/gpui-desktop
```

The script produces one `.deb` and one `.rpm`. Both install:

- `/usr/bin/mitsuro-desktop`
- the `io.mitsuro.desktop` launcher and AppStream metadata
- the canonical application icon
- GPUI SVG assets under `/usr/share/mitsuro-gpui-desktop/assets`
- the MIT license

The binary resolves packaged assets relative to its executable, with
`MITSURO_GPUI_ASSET_DIR` available for portable/test layouts. Debian packages declare
GTK 3, WebKitGTK 4.1, xkbcommon, and `xdg-utils`; RPM packages declare their distro
equivalents.

The desktop package does not embed a Mitsuro server or Codex CLI. `mitsuro-http`
connects to the configured Mitsuro service, while `codex-stdio` requires a compatible
Codex executable. This keeps backend ownership explicit and allows the same installed
client to trade between them.

## Legacy shell boundary

`apps/desktop/shell` and `apps/desktop/ui` retain the earlier Tauri/Expo implementation
for migration and comparison. They are not the canonical tagged Linux desktop artifact.
Their WebKit data-directory migration receipts do not describe GPUI state; GPUI owns
only its documented preference file and backend-owned session data.
