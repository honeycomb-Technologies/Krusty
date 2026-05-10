# Desktop Shell (Tauri)

This wraps the Expo web app surface as a native desktop app.

## Dev Flow
1. Run desktop shell (`bun run dev`) in this folder.
2. Tauri will automatically run the Expo web dev server from `apps/mobile`.

The shell loads `http://localhost:5173` during development.

## Linux Build Flow
1. Install dependencies:
   - Bun 1.3+
   - Rust stable
   - GTK/WebKit build dependencies for your distro (`webkit2gtk`, `gtk3`, `libayatana-appindicator`, `patchelf`)
2. Build packages in this folder:
   - `bun install`
   - `bun run build`

`tauri.conf.json` points `frontendDist` at `../../mobile/dist`.

Build outputs:
- `src-tauri/target/release/bundle/deb/*.deb`
- `src-tauri/target/release/bundle/rpm/*.rpm`

## Linux Install + Run
- Debian/Ubuntu:
  - `sudo apt install "./src-tauri/target/release/bundle/deb/Krusty Desktop_0.6.2_amd64.deb"`
- Fedora/RHEL:
  - `sudo dnf install "./src-tauri/target/release/bundle/rpm/Krusty Desktop-0.6.2-1.x86_64.rpm"`
- openSUSE:
  - `sudo zypper install "./src-tauri/target/release/bundle/rpm/Krusty Desktop-0.6.2-1.x86_64.rpm"`

After install, launch with:
- `krusty-desktop`

If your Wayland compositor has dmabuf issues, force X11 fallback:
- `GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 krusty-desktop`

## Bundle Notes
- `bun run build` / `bun run build:linux` creates Linux `.deb` and `.rpm`.
- `bun run build:all` attempts all bundle formats.
- `bun run build:appimage` builds AppImage only and requires `linuxdeploy`.
