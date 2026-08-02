# Mitsuro JS/TS Demo Plugin

A minimal npm-shaped TypeScript plugin package for Mitsuro's edon/libnode TUI host.

Install from the repo root:

```text
/plugins install ./examples/plugins/js-ts
/plugins enable js-ts-demo
```

To run it, Mitsuro needs libnode:

```bash
export MITSURO_LIBNODE=/path/to/libnode.so
# or EDON_LIBNODE_PATH=/path/to/libnode.so
```

The entry file calls `mitsuro.registerPlugin({ renderText, tick, onActivate })`. Mitsuro evaluates `.ts` entries through edon's TypeScript path and renders the returned text lines.
