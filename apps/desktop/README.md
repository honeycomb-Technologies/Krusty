# Mitsuro Desktop

Desktop-first Mitsuro product surfaces.

## Layout

- `gpui/` — canonical native desktop client and Linux release package; supports
  Mitsuro HTTP/SSE and a managed Codex app-server as first-class transports
- `ui/` — legacy Expo desktop product UI retained during migration
- `shell/` — legacy Tauri host retained for migration and forensic comparison; it
  is no longer the tagged Linux desktop release artifact

## Dev

```bash
cargo run -p mitsuro-gpui-desktop
```

The GPUI client consumes the transport-neutral Rust desktop backend and never imports
fixture records into a production connection. See [`gpui/README.md`](gpui/README.md)
for backend selection, validation, and capability boundaries.
