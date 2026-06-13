# Native Rust Demo Plugin

This is a minimal npm-shaped Krusty native plugin package.

## Build

```bash
cd examples/plugins/native-rust
npm run build
```

The build copies the cdylib to the path declared by `plugin.toml`:

```text
dist/linux-x64/libkrusty_native_rust_demo.so
```

## Install locally

From the Krusty repo root:

```text
/plugins install ./examples/plugins/native-rust
```

Open the plugin window and select **Native Rust Demo**.

## Hot reload

Edit `src/lib.rs`, rebuild, then reload the active plugin shell:

```bash
cd examples/plugins/native-rust
npm run build
```

```text
/plugins reload native-rust-demo
```

Krusty shadow-copies the dylib before loading it, so the rebuilt library can replace the source file while the previous copy remains loaded until the old plugin instance drops.
