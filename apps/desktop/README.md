# Mitsuro Desktop

Desktop-first Mitsuro product surface (workstation shell).

## Layout

- `ui/` — desktop product UI (Expo web, plane rails, browser/terminal panes, Hive plane)
- `shell/` — Tauri host, packaging, embedded/reused server bootstrap
- `gpui/` — experimental native GPUI client; shares the Mitsuro server contract and also supports Codex app-server

## Dev

```bash
cd apps/desktop/ui
bun install
bun run web
```

Or full native shell:

```bash
cd apps/desktop/shell
bun install
bun run dev
```

Desktop UI owns plane rail / context rail / canvas / utility host.
Mobile remains mobile-first and is reused as a component source, not as the desktop shell.

The GPUI client is a separately implemented native surface while its architecture is
stabilized. It must consume shared server/client contracts rather than duplicate Mitsuro
business logic. It does not replace the shipped Tauri shell until it reaches an explicit
feature, accessibility, packaging, and migration gate.
