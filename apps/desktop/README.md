# Mitsuro Desktop

Desktop-first Mitsuro product surface (workstation shell).

## Layout

- `ui/` — desktop product UI (Expo web, plane rails, browser/terminal panes, Hive plane)
- `shell/` — Tauri host, packaging, embedded/reused server bootstrap

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
