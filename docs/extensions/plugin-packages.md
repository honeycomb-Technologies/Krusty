# Krusty Plugin Packages

Krusty plugin packages are npm-shaped packages that declare one or more Krusty plugins. This follows Pi's package/resource-loader model while keeping Krusty's execution backends explicit.

## Package manifest

A package declares plugin manifests in `package.json`:

```json
{
  "name": "@krusty/example-plugin",
  "version": "1.0.0",
  "krusty": {
    "plugins": ["./plugin.toml"]
  }
}
```

If `package.json` does not contain `krusty.plugins`, Krusty falls back to `./plugin.toml` when present.

## Plugin manifest

`plugin.toml` declares runtime, entrypoint, capabilities, and compatibility:

```toml
manifest_version = 1
id = "example"
name = "Example"
version = "1.0.0"
publisher = "example.publisher"
runtime = "native" # native | wasm | js
entry_component = "dist/linux-x64/libexample_plugin.so"
render_capabilities = ["text"]

[compat]
krusty_min = "0.7.0"
```

Standalone manifest installs may include signed `[release]` metadata. Package installs do not require `[release]` because npm/local package installation is the distribution boundary.

## Install

Local package directory:

```text
/plugins install ./examples/plugins/native-rust
```

npm package:

```text
/plugins install npm:@krusty/example-plugin
/plugins install npm:@krusty/example-plugin@1.2.3
```

Krusty installs npm packages under its plugin root using `npm install --prefix` and then reads `package.json` `krusty.plugins`.

Krusty captures `npm install` and package `npm run build` stdout/stderr instead of letting child processes write directly to the terminal. Failed installs include the captured output in the error message; successful installs keep the TUI clean.

## Plugin directory / catalog

The `/plugins` popup includes installed plugins plus an official, searchable catalog seeded from `docs/extensions/catalog.json`. The new `apps/website` Svelte/Bun site also publishes the same catalog at `/plugin-catalog.json` for the future `krusty.dev` relaunch. Press `/` in the popup to search; press `Enter` on an installed plugin to enable/disable it or on a catalog plugin to install its package reference.

Additional catalogs can be hosted as static JSON or TOML files in git, on a website, or behind any HTTPS URL:

```json
{
  "version": 1,
  "plugins": [
    {
      "id": "example",
      "name": "Example",
      "version": "1.0.0",
      "publisher": "example.publisher",
      "package": "npm:@krusty/example-plugin",
      "runtime": "native",
      "description": "Example searchable plugin listing",
      "tags": ["example"],
      "official": false
    }
  ]
}
```

Register a catalog source with:

```text
/plugins add-source https://example.com/krusty-plugin-catalog.json example
/plugins catalog
```

## Runtimes

### native

Native plugins are dynamic libraries loaded through Krusty's C ABI. They are unsafe by design and are equivalent to executing arbitrary local code.

Rules:

- Export `krusty_plugin_entry`.
- Return a `KrustyNativePluginV1` function table.
- Do not expose Rust trait objects across the dylib boundary.
- Keep persistent application state in the Krusty host when hot reload must preserve it.
- Treat plugin `Drop`/reload as shell lifecycle, not necessarily runtime shutdown.

Native reload uses a shadow copy of the entry dylib in `.krusty-shadow/` under the package/install root. This lets a source dylib be rebuilt while the old loaded copy remains mapped by the OS.

### wasm

`runtime = "wasm"` is the default and reserved for sandboxed TUI plugin execution through Krusty's existing Wasmtime infrastructure. Manifest/package discovery is implemented; full TUI execution is still a backend integration task.

### js

`runtime = "js"` runs JavaScript and TypeScript entry files through edon/libnode. This follows the same proof used in Pi's GPUI/libnode workspaces: load libnode dynamically, evaluate JS/TS through edon, and keep npm as the package boundary. Krusty looks for libnode at `KRUSTY_LIBNODE` first, then `EDON_LIBNODE_PATH`.

JS/TS plugins register a small text-mode TUI object:

```ts
(globalThis as any).krusty.registerPlugin({
  tick() {},
  onActivate() {},
  onDeactivate() {},
  renderText() {
    return ["Hello from TypeScript"];
  }
});
```

Krusty evaluates `.ts`, `.tsx`, `.mts`, and `.cts` entries with edon's TypeScript evaluator and `.js` entries with the CommonJS evaluator. This is intentionally small for the first pass: text rendering and lifecycle hooks are supported; richer host callbacks/input APIs can be added once the runtime contract stabilizes.

## Reload

```text
/plugins reload <plugin-id>
```

For an active native plugin, reload drops the old plugin shell, shadow-copies the current entry dylib, loads the fresh copy, and recreates the plugin instance.

For wasm/js plugins, reload currently refreshes descriptors and reinstantiates the placeholder host until those runtimes are wired.

## Example

See:

- `examples/plugins/native-rust/` for a minimal Rust cdylib plugin package.
- `examples/plugins/js-ts/` for a minimal edon/libnode TypeScript plugin package.
