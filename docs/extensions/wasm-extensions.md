# WASM Extension System

Krusty supports a WebAssembly-based extension system that lets third-party code run inside the application without risking the stability or security of the host process. Extensions are compiled to WASM components, loaded at runtime, and executed in a sandboxed virtual machine. They can add language servers, slash commands, context servers, debug adapters, documentation providers, and more.

## Why WebAssembly

Traditional plugin systems face a fundamental tension: give plugins too much access and a buggy extension can crash the whole application; give them too little and they become useless. WebAssembly resolves this by providing a fast, portable execution environment with strict isolation guarantees.

**Safety.** A WASM module runs in its own linear memory space. It cannot read or write host memory, call arbitrary system functions, or access files outside of its designated working directory. If an extension panics, the host catches the trap and continues running.

**Performance.** Wasmtime compiles WASM to native machine code ahead of time. Extensions execute at near-native speed, and Krusty caches compiled artifacts using an incremental compilation cache (capped at 64 MB) so subsequent loads are fast.

**Portability.** A single `.wasm` binary works on Linux, macOS, and Windows across both x86-64 and AArch64 architectures. Extension authors compile once and distribute everywhere.

**Deterministic resource limits.** The host uses Wasmtime's epoch interruption mechanism to prevent runaway extensions. An epoch ticker advances every 100 milliseconds, and each extension store is configured with a deadline of one tick. If an extension exceeds its time slice it yields back to the host rather than blocking the event loop.

## Zed Compatibility

The extension system is ported from Zed's `crates/extension` and `crates/extension_host` modules, adapted for a tokio async runtime instead of Zed's gpui runtime. The WIT interface definitions, manifest format, and API versioning scheme are all compatible with Zed's extension ecosystem. This means existing Zed extensions that compile to WASM component binaries can be loaded by Krusty without modification, provided the API version they target is supported.

The WIT files in the `wit/` directory at the project root mirror Zed's public interface. Internally, versioned copies of these WIT files live under `crates/krusty-core/src/extensions/wit/` in directories named by version (e.g., `since_v0.8.0/`).

## WIT: The Contract Between Host and Extension

WIT stands for WebAssembly Interface Types. It is a language-neutral schema that defines the functions, types, and resources that the host provides to extensions (imports) and the functions that extensions must implement (exports).

The top-level WIT package is `zed:extension`, and the primary world is called `extension`. Within that world, several interfaces are imported for extensions to call, and several functions are exported for the host to invoke.

### Imported Interfaces (What Extensions Can Use)

**http-client** -- Make HTTP requests. Extensions can call `fetch` with an `HttpRequest` containing method, URL, headers, optional body, and a redirect policy. The host executes the request on behalf of the extension and returns the response headers and body. A streaming variant (`fetch-stream`) is also defined in the interface for incremental responses.

**github** -- Query GitHub releases. Two functions are available: `latest-github-release` retrieves the most recent release for a repository (with options for filtering by assets and pre-release status), and `github-release-by-tag-name` fetches a specific tagged release. Extensions typically use these to download language server binaries.

**platform** -- Detect the host OS and CPU architecture. The `current-platform` function returns a tuple of the operating system (mac, linux, windows) and architecture (aarch64, x86, x86-64). Extensions use this to pick the right binary when installing tools.

**nodejs** -- JavaScript runtime access. Despite the interface name, Krusty backs this with Bun rather than Node.js. The interface exposes `node-binary-path` (returns the path to the Bun binary), `npm-package-latest-version`, `npm-package-installed-version`, and `npm-install-package`. Extensions that manage JS-based language servers use these to install and update npm packages.

**process** -- Run shell commands. The `run-command` function takes a command name, arguments, and environment variables, executes the process on the host, and returns the exit status, stdout, and stderr. Available since API version 0.3.0.

**Core extension imports** -- The extension world itself imports several utility functions directly: `download-file` downloads a URL to the extension's working directory and extracts it based on file type (gzip, tar.gz, zip, or uncompressed); `make-file-executable` sets the executable bit on a file; `set-language-server-installation-status` reports progress back to the host; and `get-settings` reads configuration values.

### Exported Functions (What Extensions Must Implement)

Every extension must export `init-extension`, called once when the extension is first loaded. Beyond that, extensions export functions based on what they provide:

- `language-server-command` -- return the command to start a language server
- `language-server-initialization-options` / `language-server-workspace-configuration` -- provide LSP configuration as JSON
- `complete-slash-command-argument` / `run-slash-command` -- handle slash commands
- `context-server-command` / `context-server-configuration` -- manage context servers
- `suggest-docs-packages` / `index-docs` -- power documentation indexing
- `get-dap-binary` / `run-dap-locator` -- configure debug adapters

### Resources

WIT resources are handle types that the host creates and passes to extension calls. The extension cannot forge or inspect them directly; it can only call methods on them through the defined interface.

- **worktree** -- represents a project directory. Methods include `id`, `root-path`, `read-text-file`, `which` (find a binary on PATH), and `shell-env`.
- **project** -- represents the overall project. Exposes `worktree-ids` to list all worktrees.
- **key-value-store** -- a simple storage handle with an `insert` method, used during documentation indexing.

## The Host Runtime

The WASM host is implemented in `crates/krusty-core/src/extensions/wasm_host/`. At its center is the `WasmHost` struct, which owns a Wasmtime `Engine`, an HTTP client, a Bun runtime, and a working directory for extension file operations.

When loading an extension, the host reads `extension.toml` from the extension directory, locates the `.wasm` file (named after the extension ID, or `extension.wasm` as a fallback), parses the `zed:api-version` custom section from the WASM binary to determine which API version to use, compiles the component, and spawns a dedicated tokio task for the extension's lifetime.

Each extension runs on its own task with its own `Store<WasmState>`. The store holds the WASI context (providing stdio, environment variables, and preopened directories), the resource table, and a reference back to the host. Communication between the host and the extension task happens over an unbounded mpsc channel: the host sends closures that are executed against the extension instance and store, with results returned through oneshot channels.

The WASI sandbox is configured to give each extension access to its own working directory (under `~/.krusty/extensions/work/<extension-id>/`) both as `.` and as the absolute path. Extensions cannot access files outside this directory through the WASI filesystem.

## Extension Manifest

Every extension includes an `extension.toml` file that declares its identity and capabilities. The format is compatible with Zed's extension manifest.

```toml
id = "my-extension"
name = "My Extension"
version = "0.1.0"
description = "A useful extension"
repository = "https://github.com/user/my-extension"
authors = ["Author Name"]

[lib]
kind = "Rust"
version = "0.8.0"

[language_servers.my-lsp]
language = "rust"

[slash_commands.my-command]
description = "Does something useful"
requires_argument = true
```

The manifest supports registering multiple component types:

- **language_servers** -- LSP servers, with language bindings and code action kinds
- **slash_commands** -- interactive commands with descriptions and argument requirements
- **context_servers** -- MCP context server configurations
- **grammars** -- Tree-sitter grammars, referenced by git repository and revision
- **indexed_docs_providers** -- documentation indexing providers
- **debug_adapters** / **debug_locators** -- DAP support for debugging
- **agent_servers** -- AI agent integration points
- **themes**, **icon_themes**, **languages**, **snippets** -- static asset paths

The `lib.version` field is critical. It determines which WIT API version the host uses when instantiating the extension, which in turn controls what functions are available.

## API Versioning

The extension system supports ten API versions, from 0.0.1 through 0.8.0. Each WASM binary embeds its target API version in a custom section named `zed:api-version`, encoded as three big-endian `u16` values (major, minor, patch).

When the host loads an extension, it reads this custom section and selects the appropriate linker and bindings. The version resolution walks from newest to oldest: if the extension's version is >= 0.8.0, it uses the latest bindings; if >= 0.6.0, it uses the 0.6.0 bindings; and so on down to 0.0.1.

Each version adds capabilities. Some highlights from the progression:

- **0.0.1** -- baseline: language server commands, GitHub API, platform detection
- **0.0.6** -- added Node.js/npm integration
- **0.1.0** -- added HTTP client, slash commands, common types
- **0.3.0** -- added process execution (run-command)
- **0.5.0** -- added context server support
- **0.6.0** -- added DAP (debug adapter protocol) support
- **0.8.0** -- added additional language server configuration, context server configuration, and extended DAP capabilities

Older extensions continue to work because the host translates their types into the latest internal representation using `Into` conversions. An extension compiled against 0.0.4, for example, will have its `Command` type automatically converted to the current format.

## The Bun Runtime

Some extensions, particularly those wrapping JavaScript-based language servers, need a JS runtime to install and run npm packages. Krusty uses Bun instead of Node.js for this purpose, chosen for its faster startup and install times.

The `BunRuntime` manager in `crates/krusty-core/src/extensions/bun_runtime.rs` handles Bun lifecycle:

1. **System detection** -- first checks if `bun` is available on the system PATH
2. **Managed installation** -- if no system Bun is found, downloads Bun 1.1.42 from GitHub releases, extracts it, and places it in the Krusty data directory
3. **Caching** -- the detected or installed instance is cached behind an `Arc<RwLock>` so subsequent calls avoid re-detection

When extensions call the `nodejs` WIT interface, the host routes those calls through BunRuntime. `node-binary-path` returns the path to the Bun binary. `npm-install-package` runs `bun add`. Package version queries use `bun pm info`. From the extension's perspective, the interface is identical to what Zed provides with Node.js.

## Where Extensions Live

Extensions are stored under `~/.krusty/extensions/`. The directory layout separates installed extension files from runtime working directories:

```
~/.krusty/extensions/
    my-extension/
        extension.toml      # manifest
        my-extension.wasm   # compiled WASM component
        languages/          # optional language definitions
        themes/             # optional themes
    work/
        my-extension/       # sandboxed working directory
            node_modules/   # if the extension installs npm packages
            ...             # downloaded binaries, caches, etc.
```

The `work/` subdirectory is where WASI filesystem access is rooted. When an extension calls `download-file` with a relative path, the file ends up under `work/<extension-id>/`. Path traversal attempts (using `..`) are normalized away before any filesystem operation.

## Extension Installation from GitHub

The `github.rs` module provides functions for fetching release metadata from the GitHub API. Extensions that need to download language server binaries use the `latest-github-release` WIT function, which calls through to this module. The host automatically includes a `GITHUB_TOKEN` from the environment if one is set, avoiding rate limits during development and CI.

Release queries support filtering by whether assets are present and whether pre-releases should be included, making it straightforward for extensions to find the right binary for the current platform.
