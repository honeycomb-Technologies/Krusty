# Building, CI/CD & Packaging

This document explains how Krusty is built, tested, and distributed across all of its targets: the Rust CLI, the self-hosted server, the Expo mobile app, and the Tauri desktop shell. It covers the workspace layout, the quality gates enforced in CI, how releases are cut, and the various channels through which users can install Krusty.

## Rust workspace

The Rust side of Krusty is organized as a Cargo workspace with eight Krusty crates plus the `grok-auth` support crate. The primary runtime boundaries are:

- **krusty-cli** (`crates/krusty-cli`) -- The terminal application with the TUI. This is the default member of the workspace, so a bare `cargo build` compiles it. It depends on both `krusty-core` and `krusty-server`.
- **krusty-core** (`crates/krusty-core`) -- The core library containing AI provider integrations, tool implementations, the ACP/MCP protocol layers, WASM extension hosting, and local storage. Everything shared between the CLI and the server lives here.
- **krusty-server** (`crates/krusty-server`) -- The self-hosted API server built on Axum. It serves the REST/WebSocket APIs used by the mobile and desktop clients, and optionally embeds the web frontend (more on that below).
- **krusty-client** and **krusty-client-state** -- Typed transport and shared client-state boundaries.
- **krusty-desktop**, **krusty-mobile**, and **krusty-mobile-ui** -- Native desktop/mobile presentation crates that consume the same core contracts.

The workspace root `Cargo.toml` sets a few important release profile options: link-time optimization (`lto = true`), a single codegen unit (`codegen-units = 1`), and symbol stripping (`strip = true`). These produce smaller, faster release binaries at the cost of longer compile times. The workspace also defines shared lint rules so all three crates enforce the same code quality standards through Clippy.

The eight Krusty crates and desktop bundle share version `0.7.3` and edition 2021.

## Build commands

Four commands must pass before any code is committed or released:

```bash
cargo fmt --all              # Format all crates
cargo clippy --workspace -- -D warnings   # Lint with warnings as errors
cargo build --workspace      # Compile all three crates
cargo test --workspace       # Run all tests
```

`cargo fmt` enforces consistent formatting. `cargo clippy` catches common mistakes and enforces the workspace lint configuration, which warns on dead code, unused imports, redundant clones, and unnecessary wraps. The build and test steps ensure everything compiles and the test suite passes.

For release builds, `cargo build --release` enables the LTO and stripping profile described above. Cross-compilation for other architectures uses the `cross` tool (see below).

## CI pipeline

The CI workflow (`.github/workflows/ci.yml`) runs on every push to `main` and on every pull request targeting `main`. It breaks the quality gates into parallel jobs so failures surface quickly:

| Job | What it does |
|---|---|
| **Rust Check** | `cargo check --workspace` -- fast compilation check without producing binaries |
| **Rust Test** | `cargo test --workspace` -- runs the full test suite |
| **Rust Clippy** | `cargo clippy --workspace -- -D warnings` -- lint pass with zero tolerance for warnings |
| **Rust Format** | `cargo fmt --all -- --check` -- verifies formatting without modifying files |
| **Client Quality Gates** | Calls the reusable client workflow: Bun API contract tests, Deno shared-state type-check/tests, mobile TypeScript, and Expo web export |
| **Desktop Linux Bundle** | Full Tauri build on Ubuntu with system dependencies (WebKit, GTK, patchelf) |

The Linux jobs install `libudev-dev` because the CLI uses `gilrs` for gamepad support, which requires the udev headers. The desktop bundle job additionally installs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, and `patchelf` for the Tauri build.

All Rust jobs use `dtolnay/rust-toolchain@master` pinned to the stable channel. A PR cannot merge until every job is green.

### Default-branch and governance preflight

Before routing autonomous continuation, opening broad implementation work, or validating release/TestFlight governance, run the read-only default-branch preflight from the repository root:

```bash
scripts/check-default-branch-preflight.sh
```

The preflight compares the GitHub default branch reported by `gh repo view`, the authoritative remote HEAD from `git ls-remote --symref origin HEAD`, and the local tracking symref at `refs/remotes/origin/HEAD`. Krusty's expected default branch is `main`; if local `origin/HEAD` still points at a stale branch such as `origin/dev`, fix the local ref before using it to choose a base branch. The check is ruleset-aware: a classic branch-protection 404 is acceptable only when an active branch ruleset applies to `refs/heads/main`.

The preflight is safe for validation: it reads repository metadata, remote refs, classic branch protection, and repository rulesets only. It does not push, delete branches, trigger workflows, edit settings, create releases, or read secret values.

## Release automation

Releases are triggered by pushing a Git tag that matches `v*` (for example, `v0.7.3`). Before packaging, the release workflow calls `.github/workflows/client-quality.yml`; no release build starts unless the API streaming/error contracts, shared-state type-check/tests, mobile TypeScript check, and Expo web export all pass. The remaining release workflow (`.github/workflows/release.yml`) has three artifact stages:

**1. Build matrix.** A matrix job compiles release binaries for five targets:

| Target | Runner |
|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` (native ARM runner) |
| `x86_64-apple-darwin` | `macos-15-intel` |
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-pc-windows-msvc` | `windows-latest` |

Each job first builds the Expo web frontend from `apps/mobile` (so it can be embedded into the server and desktop bundles), then compiles Krusty in release mode for the target architecture. Unix builds are packaged as `.tar.gz` archives; the Windows build is packaged as a `.zip`.

**2. Desktop Linux bundles.** A separate job builds the Tauri desktop shell on Ubuntu, producing `.deb` and `.rpm` packages from `apps/desktop/shell`.

**3. Create release.** Once both build stages complete, all artifacts are downloaded and a GitHub Release is created with auto-generated release notes. The release includes the CLI binaries for all five platforms plus the desktop Linux packages.

The publish job fails closed unless GitHub reports the release tag as protected (`github.ref_protected == true`). The repository also maintains an active `Protect release tags` ruleset. Do not push a `v*` tag merely to test the workflow; create one only for an approved, fully verified release.

## Distribution channels

### Install script

The fastest way to install Krusty is the one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/honeycomb-Technologies/Krusty/main/install.sh | sh
```

The script (`install.sh`) detects the host OS and architecture, fetches the latest release from the GitHub API, downloads the correct archive, verifies its SHA-256 checksum when available, extracts the binary, and copies it to `~/.local/bin` (configurable via `INSTALL_DIR`). It supports Linux (x86_64, aarch64), macOS (Intel and Apple Silicon), and Windows under MSYS/Cygwin. If the install directory is not already in `PATH`, it prints the line you need to add to your shell config.

You can pin a specific version by setting `VERSION` before running the script:

```bash
VERSION=v0.7.3 curl -fsSL ... | sh
```

## Self-hosted systemd service

The checked-in user services run the HTTP control plane and the independent
Mako autonomous backend from binaries on `PATH`. The HTTP server requires
Mako's private socket and fails closed if it cannot complete an authenticated
daemon handshake. Build both release binaries before installing or restarting
the units:

```bash
cd apps/mobile && bun install --frozen-lockfile && bun run web:build && cd ../..
cargo build --release -p krusty -p krusty-mako
install -d -m755 ~/.config/systemd/user
install -m644 deploy/systemd/krusty-mako.socket deploy/systemd/krusty-mako.service \
  deploy/systemd/krusty-serve.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now krusty-mako.socket krusty-serve.service
systemctl --user is-active krusty-mako.socket krusty-mako.service krusty-serve.service
curl --fail http://127.0.0.1:3000/health
```

`krusty-mako.socket` starts the daemon on demand. Do not run a second manual
daemon against the same database and socket. For deployment verification,
confirm the three tracked unit states, an authenticated Mako diagnostics/API
request through the server, and a restart-recovery test; a successful Cargo
build alone is not proof that autonomous work is live.

Release archives include these user units, the shell installer places them in
`~/.config/systemd/user`, and the AUR package places them in
`/usr/lib/systemd/user`. The default hardening grants writes under `~/Work` and
Krusty's state/cache directories; add a user-service drop-in with an additional
`ReadWritePaths=` entry when autonomous sessions use another project root.
Homebrew exposes `krusty-mako` through `brew services`; the HTTP server remains
an explicitly configured self-host service.

Mako resolves `.krusty/skills` independently for each run's frozen project
root. Autonomous daemon runs intentionally do not load project `.mcp.json`
servers yet, and MCP connections made through the HTTP `/mcp` API do not cross
the process boundary into the daemon. This is fail-closed: project MCP will be
enabled only with durable project/config trust and explicit MCP process
lifecycle ownership, rather than accidentally exposing the daemon launch
directory's tools to every project.

### Homebrew tap

macOS and Linux users with Homebrew can install from the tap:

```bash
brew install BurgessTG/tap/krusty
```

The formula (`.github/homebrew/krusty.rb`) selects the correct binary archive based on CPU architecture. It supports macOS ARM64, macOS x86_64, Linux ARM64, and Linux x86_64. The version, URLs, and SHA-256 checksums use placeholders that are updated by CI automation on each release.

### AUR package

Arch Linux users can install from the AUR. The `PKGBUILD` (`aur/PKGBUILD`)
downloads the source tarball for a given release tag, verifies its pinned SHA-256
checksum, builds from source using Cargo with the stable toolchain, runs the test
suite during the `check()` phase, and installs the Krusty and Mako binaries,
their systemd user units, and the license. It supports both `x86_64` and
`aarch64` architectures. Runtime dependencies are `gcc-libs` and `openssl`; the
only build dependency is `cargo`.

GitHub does not publish the versioned source archive until its tag exists, so
AUR metadata is updated after the protected release tag is published. Run:

```bash
./aur/update-release.sh 0.8.0
cd aur
makepkg --verifysource -f
makepkg --printsrcinfo | diff -u .SRCINFO -
```

The updater downloads over HTTPS, validates the archive's top-level directory,
computes SHA-256 locally, and updates both `PKGBUILD` and `.SRCINFO`. Review and
publish those two files to the AUR repository. Never replace the checksum with
`SKIP`; if a release tag or its archive changes, checksum verification must fail
until a maintainer explicitly reviews and updates the package metadata.

## Cross-compilation

The `Cross.toml` file configures the `cross` tool for building on architectures different from the host. Currently it defines a single target:

```toml
[target.aarch64-unknown-linux-gnu]
pre-build = [
    "dpkg --add-architecture arm64",
    "apt-get update && apt-get install -y libudev-dev:arm64",
]
```

This ensures the ARM64 udev headers are available inside the cross-compilation container. The `krusty-core` crate vendors both libgit2 and OpenSSL (via the `vendored-libgit2` and `vendored-openssl` features on the `git2` dependency) specifically to make cross-compilation reliable -- these libraries are compiled from source rather than linked against system copies that may not exist for the target architecture.

In the release workflow, the ARM64 Linux build runs on a native `ubuntu-24.04-arm` GitHub runner rather than using cross-compilation, which avoids the container overhead entirely.

## Mobile builds

The mobile app lives in `apps/mobile` and is built with Expo and EAS (Expo Application Services). The `eas.json` file defines three build profiles:

- **development** -- Builds a development client with internal distribution. On iOS, this targets the simulator.
- **preview** -- Builds for internal distribution (ad-hoc provisioning), used for testing on physical devices before a release.
- **production** -- Builds for App Store distribution with auto-incrementing iOS build numbers.

### TestFlight deployment

The `mobile-testflight.yml` workflow automates iOS builds and TestFlight submission. It triggers on pushes to `main` when files change under `apps/mobile/**`, `packages/**`, or `.github/workflows/mobile-testflight.yml`. The workflow can also be triggered manually via `workflow_dispatch`; choose `main` in the GitHub UI/CLI because Krusty's default branch and release governance are anchored on `main`, not on the legacy `dev` branch.

Push path filters are intentionally narrow so Rust-only, docs-only, and desktop-only changes do not start a TestFlight build. Manual `workflow_dispatch` runs do not get the same path-filter protection, so use manual dispatch only for an intentional TestFlight validation on `main`, after reviewing the diff and approvals.

The job targets the GitHub Actions `testflight` environment (`environment: testflight`). Keep that environment configured with human approval/reviewer protection so EAS build and App Store Connect secrets are not exposed and the build is not submitted until the approval gate is satisfied.

The workflow uses concurrency control (`group: mobile-ios-build, cancel-in-progress: true`) so that a new push cancels any in-flight build rather than queueing up stale builds. It first calls the same reusable client-quality workflow used by CI and releases. The TestFlight build job cannot start until the API contract tests, shared-state type-check/tests, mobile TypeScript check, and Expo web export pass. The build and submission steps are:

1. Check out the repo and install Bun
2. Install dependencies with `bun install --frozen-lockfile`
3. Set up EAS with the `expo/expo-github-action`
4. Run `eas build --platform ios --profile production --non-interactive`
5. Submit the resulting build to TestFlight with `eas submit`

The App Store Connect app ID (`6761496828`) is configured in `eas.json`. Apple credentials are stored as GitHub secrets.

For safe no-TestFlight validation of docs/tooling changes, inspect the workflow definition and repository metadata only (for example, `gh workflow view mobile-testflight.yml --repo honeycomb-Technologies/Krusty`, `scripts/check-default-branch-preflight.sh`, and `git diff --check`). Do not validate this path by pushing mobile changes to `main`, running `gh workflow run mobile-testflight.yml`, or submitting a build unless Jacob/Bob explicitly approves a TestFlight release attempt.

## Desktop builds

The desktop app is a Tauri v2 shell (`apps/desktop/shell`) that wraps the same React frontend used by the mobile app. The Tauri process also starts the embedded Krusty server, so the desktop app is fully self-contained -- it does not require a separate server process.

The `tauri.conf.json` configures the build pipeline:

- **Before build:** Runs `npx expo export --platform web` in the mobile app directory, which produces a static web export in `apps/mobile/dist`.
- **Frontend dist:** Points to `../../mobile/dist`, so Tauri bundles those static assets into the native binary.

Build scripts in `apps/desktop/shell/package.json`:

```bash
bun run build          # Produces .deb and .rpm packages
bun run build:linux    # Same as above, explicit Linux target
bun run build:all      # All Tauri bundle targets
bun run build:appimage # AppImage (requires linuxdeploy)
```

The Tauri shell depends on `krusty-server` and `krusty-core` directly via path references in its `Cargo.toml`, so the Rust server is compiled into the desktop binary alongside the Tauri runtime.

The bundle configuration targets the `DeveloperTool` category and includes icons at multiple resolutions (32px through 512px). Linux-specific settings put the `.deb` package in the `utils` section with `optional` priority.

## The embedded web frontend

The self-hosted server has a notable trick: the web frontend is compiled directly into the Rust binary at build time using the `rust-embed` crate. In `krusty-server/src/lib.rs`, the `WebAssets` struct is annotated with `#[derive(Embed)]` and pointed at `apps/mobile/dist`:

```rust
#[derive(Embed)]
#[folder = "../../apps/mobile/dist"]
#[prefix = ""]
#[allow_missing = true]
struct WebAssets;
```

The `allow_missing = true` attribute is key -- if the `dist` directory does not exist at compile time (for example, during a plain `cargo build` without running the frontend build first), the server compiles successfully but runs in API-only mode. When a browser hits the server, it receives a plain-text message instead of the web app.

For a full build (as done in the release workflow), the process is:

1. Run `bun install --frozen-lockfile` and `npx expo export --platform web` in `apps/mobile` to produce the static export in `apps/mobile/dist`.
2. Run `cargo build --release` -- rust-embed picks up the files and bakes them into the binary.

The server handles all non-API routes with SPA fallback: it tries to match the request path to an embedded file, and if nothing matches, serves `index.html` so client-side routing works. Static assets under `_expo/static/` get immutable cache headers (one year), while HTML files are served with `no-cache`.

## Development workflow

During development, you typically want the backend and frontend running simultaneously with hot reload.

### CLI-only development

If you are working on the Rust CLI or core library:

```bash
cargo run              # Runs krusty-cli (the default workspace member)
cargo run --release    # Release mode for performance testing
cargo test --workspace # Run all tests across all crates
```

### Server + web frontend

To work on the web frontend with live reload:

1. **Start the Expo dev server** in one terminal:
   ```bash
   cd apps/mobile && bun start --web
   ```

2. **Start the Rust server** in another terminal:
   ```bash
   cargo run -p krusty-server
   ```

The frontend dev server handles hot module replacement. The Rust server serves the API.

### Desktop development

Tauri provides an integrated dev experience. The `beforeDevCommand` in `tauri.conf.json` starts the Expo web dev server on port 5173, and `devUrl` points Tauri's webview at it:

```bash
cd apps/desktop/shell && bun run dev
```

This launches the Tauri window with the live-reloading frontend and the embedded Rust server. Changes to the React code reflect instantly; changes to the Rust code trigger a recompile.

### Mobile development

For the iOS/Android app:

```bash
cd apps/mobile
bun start              # Start Expo dev server
bun run ios            # Build and run on iOS simulator
bun run android        # Build and run on Android emulator
```

The mobile app connects to a running Krusty server instance (either local or remote) for its backend.
