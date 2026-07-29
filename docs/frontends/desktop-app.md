# Desktop App (Tauri)

Mitsuro ships as a desktop application for Linux (and eventually macOS and Windows) using Tauri. If you've heard of Electron — the framework behind VS Code, Slack, and Discord — Tauri solves the same problem but in a fundamentally different way. Instead of bundling an entire copy of Chromium, Tauri uses the operating system's built-in web renderer (WebKitGTK on Linux, WebKit on macOS, WebView2 on Windows). The result is a desktop app that weighs tens of megabytes instead of hundreds, uses a fraction of the RAM, and starts faster.

The desktop app doesn't contain its own UI. It wraps the same Expo/React web build that powers the mobile app and the embedded server frontend. One codebase produces the mobile app, the web frontend, and the desktop app — Tauri just provides the native window and the Rust backend that ties everything together.

## How Tauri Works

Tauri is a Rust framework for building desktop applications. You write your backend logic in Rust and your frontend in any web technology you want. At build time, Tauri compiles your Rust code into a native binary and bundles your web assets alongside it. At runtime, it opens a native window, loads your web content into the system's web view, and bridges between the JavaScript frontend and the Rust backend.

This means the Mitsuro desktop app is a native Rust binary that happens to render a React UI in a system web view. There's no embedded browser engine, no V8 runtime, no Node.js process. The Rust side handles everything that needs native access, and the web side handles rendering.

## Architecture

The desktop app lives in `apps/desktop/shell/`. Its structure follows the standard Tauri v2 layout:

```
apps/desktop/shell/
  package.json          npm/bun scripts for dev and build commands
  src-tauri/
    tauri.conf.json     window config, bundle settings, build pipeline
    Cargo.toml          Rust dependencies
    src/main.rs         application entry point
    icons/              app icons at various resolutions
```

The important relationship is between the desktop shell and the mobile app at `apps/mobile/`. The desktop shell does not contain any UI code of its own. It points directly at the mobile app's web export:

- During development, Tauri launches the Expo web dev server (`npx expo start --web --port 5173`) and loads it in the window.
- During production builds, Tauri first runs `npx expo export --platform web` to generate a static web build, then bundles those files into the native binary.

This means the Expo app is the single source of truth for the UI across mobile, web, and desktop. Changes to the React code show up everywhere.

## The Build Pipeline

A production build follows these steps:

1. **Expo web export.** Tauri's `beforeBuildCommand` runs `npx expo export --platform web` inside the `apps/mobile/` directory. This produces a static HTML/CSS/JS bundle in `apps/mobile/dist/`.

2. **Rust compilation.** Tauri compiles the Rust backend (`src-tauri/src/main.rs`) along with its dependencies: `krusty-core` for server detection, `krusty-server` for the embedded server, `tokio` for async runtime, and `tracing` for structured logging.

3. **Asset bundling.** Tauri takes the static web assets from step 1 and embeds them into the native binary.

4. **Package generation.** Tauri produces platform-specific installers. On Linux, the default build generates both `.deb` and `.rpm` packages. AppImage builds are also supported but require `linuxdeploy` to be installed separately.

The build commands in `package.json` reflect this:

- `bun run build` produces DEB and RPM packages.
- `bun run build:all` produces all available package formats.
- `bun run build:appimage` produces an AppImage (requires `linuxdeploy`).

Output packages land in `apps/desktop/shell/src-tauri/target/release/bundle/`.

## Window Configuration

The Tauri config at `src-tauri/tauri.conf.json` defines the window properties:

| Property | Value |
|----------|-------|
| Title | Mitsuro Desktop |
| Default size | 1280 x 860 |
| Minimum size | 1024 x 680 |
| Resizable | Yes |

The default size gives comfortable room for the chat interface, sidebar, and code blocks without feeling cramped. The minimum dimensions prevent the layout from breaking at very small sizes. The window is freely resizable above the minimum.

The `withGlobalTauri` option is enabled, which exposes Tauri's JavaScript API on the `window.__TAURI__` global. This allows the Expo web app to call into Tauri's native capabilities if needed, though the current implementation keeps the bridge minimal.

## How It Connects to the Server

This is where the desktop app does something clever. When the app launches, its Rust backend doesn't just open a window — it ensures a Mitsuro server is running.

The startup sequence in `main.rs` works like this:

1. **Check for an existing server.** The app calls `server_instance::detect_running_server()`, which looks for an already-running Mitsuro server process. If you started `krusty serve` in a terminal before launching the desktop app, it finds that instance and reuses it.

2. **Start an embedded server if needed.** If no server is running, the app starts one inside its own process. It prefers port 3000 but will fall back to a random available port if 3000 is taken. The server launches in a background tokio task and writes a PID file so other Mitsuro instances can discover it.

3. **Wait for health.** The app polls the server's health endpoint for up to 5 seconds (50 attempts at 100ms intervals) before proceeding. This ensures the server is ready to accept requests before the UI tries to connect.

4. **Inject the connection URL.** Once the server is ready, Tauri's `setup` hook runs a small JavaScript snippet in the web view: `window.__KRUSTY_SERVER_URL = 'http://localhost:PORT'` and `window.__KRUSTY_SERVER_TOKEN = 'local'`. The Expo app's `useConnection` hook checks for these globals on startup. When it finds them, it skips the manual server configuration screen and connects automatically.

The result is seamless: you launch the desktop app and it's immediately connected to a Mitsuro server, either one you already had running or one it started for you. No configuration dialogs, no URLs to paste, no tokens to copy.

## Linux WebKit Workarounds

On Linux, Tauri uses WebKitGTK as its web renderer. Some Linux graphics drivers have issues with WebKit's DMA-BUF renderer, causing visual glitches or crashes. The app detects when it's running on Linux and sets the `WEBKIT_DISABLE_DMABUF_RENDERER` environment variable to work around this. The workaround is only applied if the user hasn't already set the variable themselves.

## Development Workflow

Day-to-day development uses the Tauri dev server:

```bash
cd apps/desktop/shell
bun install
bun run dev
```

This runs `tauri dev`, which:

1. Starts the Expo web dev server on port 5173 (via the `beforeDevCommand` in `tauri.conf.json`).
2. Compiles and launches the Rust backend.
3. Opens the desktop window pointing at `http://localhost:5173`.

Hot reloading works for the frontend — edit a React component and it updates in the desktop window without restarting. Changes to the Rust backend require a recompile, which Tauri handles automatically.

## Dependencies

The Rust side (`Cargo.toml`) pulls in a focused set of dependencies: **tauri** v2 as the framework, **krusty-server** and **krusty-core** so the app can start and detect server instances, **tokio** for the async runtime, and **tracing** for structured logging. The JavaScript side has a single dev dependency: `@tauri-apps/cli`, the Tauri CLI that orchestrates builds and the dev server. The project uses Bun as its package manager.

## Build Targets and Distribution

The bundle configuration in `tauri.conf.json` defines how the app is packaged:

- **Identifier:** `io.krusty.desktop`
- **Category:** DeveloperTool
- **License:** MIT
- **Publisher:** Mitsuro

On Linux, the default build produces:

- **DEB packages** for Debian/Ubuntu-based distributions, placed in the `utils` section with `optional` priority.
- **RPM packages** for Fedora/RHEL-based distributions.

The `targets: "all"` setting in the config means Tauri will produce every package format it supports for the current platform. The `package.json` scripts narrow this to DEB and RPM for standard builds, with an explicit AppImage option available.

## Why Tauri Over Electron

The choice of Tauri over Electron comes down to three factors:

**Binary size.** Electron bundles Chromium, which adds roughly 150-200 MB to every app. Tauri uses the system's existing web view, so the overhead is just the Rust binary — typically 10-30 MB. For a developer tool that users install alongside dozens of others, this matters.

**Memory usage.** An Electron app runs its own Chromium process with its own V8 isolate. A Tauri app renders in a lightweight system web view. In practice, Tauri apps use significantly less RAM, which is relevant when the desktop app will be running alongside a code editor, terminal, browser, and the Mitsuro server itself.

**Rust backend.** Mitsuro is a Rust project. Tauri's backend is Rust. This means the desktop app can import `krusty-core` and `krusty-server` directly as Cargo dependencies and call into them natively — no IPC overhead, no foreign function interface, no subprocess spawning. The embedded server runs in the same process as the desktop window. Electron would have required either a separate server process or bridging between Node.js and Rust via N-API or similar.

The trade-off is that Tauri's web view has slightly less consistent behavior across platforms compared to Electron's bundled Chromium. The WebKit workaround for Linux DMA-BUF rendering is one example. In practice, for a React-based chat interface, these differences are minor and manageable.
