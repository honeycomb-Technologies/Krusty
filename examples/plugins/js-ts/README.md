# Krusty JS/TS Demo Plugin

A minimal npm-shaped TypeScript plugin package for Krusty's edon/libnode TUI host.

Install from the repo root:

```text
/plugins install ./examples/plugins/js-ts
/plugins enable js-ts-demo
```

To run it, Krusty needs libnode:

```bash
export KRUSTY_LIBNODE=/path/to/libnode.so
# or EDON_LIBNODE_PATH=/path/to/libnode.so
```

The entry file calls `krusty.registerPlugin({ renderText, tick, onActivate })`. Krusty evaluates `.ts` entries through edon's TypeScript path and renders the returned text lines.
