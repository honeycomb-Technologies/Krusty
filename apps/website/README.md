# Krusty Website

Fresh SvelteKit/Bun website for the future `krusty.dev` relaunch.

## Commands

```bash
cd apps/website
bun install
bun run dev -- --port 5174
bun run check
bun run build
```

The build output is static and written to `apps/website/build/`.

## Plugin catalog

The website publishes the official plugin catalog at:

```text
/plugin-catalog.json
```

`bun run sync:catalog` copies the canonical repository catalog from:

```text
docs/extensions/catalog.json
```

into both:

```text
apps/website/static/plugin-catalog.json
apps/website/src/lib/data/catalog.generated.ts
```

The `dev`, `check`, and `build` scripts run the sync step automatically.
