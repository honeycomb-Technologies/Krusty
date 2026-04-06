# Desktop App

This directory is reserved for desktop packaging around the React/Expo web client.

## Scope
- Native wrapper and packaging
- OS integration (notifications, file dialogs, autostart)
- Local connection config to self-hosted `krusty-server`

## Active Scaffold
- `shell/` contains a Tauri wrapper targeting `apps/mobile`.

## Local Run

```bash
cd apps/desktop/shell
bun install
bun run dev
```

Build:

```bash
bun run build
```

Linux packages are generated at:
- `apps/desktop/shell/src-tauri/target/release/bundle/deb/*.deb`
- `apps/desktop/shell/src-tauri/target/release/bundle/rpm/*.rpm`
