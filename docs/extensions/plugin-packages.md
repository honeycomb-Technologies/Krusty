# Mitsuro Plugin Packages

Mitsuro plugin packages are npm-shaped packages that declare one or more Mitsuro plugins. This follows Pi's package/resource-loader model while keeping Mitsuro's execution backends explicit.

## Package manifest

A package declares plugin manifests in `package.json`:

```json
{
  "name": "@mitsuro/example-plugin",
  "version": "1.0.0",
  "mitsuro": {
    "plugins": ["./plugin.toml"]
  }
}
```

If `package.json` does not contain `mitsuro.plugins`, Mitsuro falls back to `./plugin.toml` when present.
An explicit list may contain at most 256 unique, normalized manifest paths.
Each manifest is limited to 1 MiB and all manifests in one package are limited
to 8 MiB in aggregate, bounding both parser work and package fan-out.

## Plugin manifest

`plugin.toml` declares identity and any combination of TUI, agent, skill, MCP,
hook, and asset components:

```toml
manifest_version = 1
id = "example"
name = "Example"
version = "1.0.0"
publisher = "example.publisher"
runtime = "native" # native | wasm | js
entry_component = "dist/linux-x64/libexample_plugin.so"
skills = ["skills/release/SKILL.md"]
agent_extensions = ["extensions/release.ts"]
mcp_servers = "mcp/servers.json"
hooks = ["hooks/hooks.json"]
assets = "assets"
render_capabilities = ["text"]

[requested_permissions]
fs_read = true
network = true
process = true

[compat]
mitsuro_min = "0.7.0"
```

`entry_component` is optional for bundle-only packages. Every component path is
relative to the package root, must exist at install time, and is canonicalized
to ensure that neither path traversal nor symlinks can escape the immutable
installed snapshot.

`hooks` accepts declarative `.json` or `.toml` command-hook configurations only.
Executable JavaScript or TypeScript belongs in `agent_extensions`. Mitsuro accepts
its flat hook form and the command subset of Codex/Claude event maps:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write",
        "hooks": [
          { "type": "command", "command": "./hooks/guard.sh", "timeout": 15 }
        ]
      }
    ]
  }
}
```

Package hooks are read-only and ephemeral: enable, disable, update, uninstall,
and permission changes replace the in-memory set without writing them to the
user hook database. Commands run from the immutable package root. Hook output
is bounded, timeouts terminate the process group, and an invalid replacement
clears package hooks rather than retaining stale executable state.

Standalone manifest installs require signed `[release]` metadata. The SHA-256
digest verifies the artifact; the Ed25519 signature authenticates a
domain-separated canonical release envelope containing identity, version,
publisher, runtime, component paths, requested permissions, compatibility,
release URL, artifact kind, digest, and signing-key ID. This prevents a valid
artifact signature from being replayed under altered manifest metadata.

```toml
[release]
url = "https://plugins.example/example.wasm"
sha256 = "<64-character-hex-digest>"
signature = "<ed25519-signature-base64>"
signing_key_id = "release-2026"
signature_scheme = "manifest-envelope-v1"
```

The backward-compatible default is a single component: omit `artifact_kind`
and declare `entry_component`. To distribute an authenticated multi-resource
bundle, publish a ZIP whose root is the plugin package root and opt into the
container explicitly:

```toml
skills = ["skills/example/SKILL.md"]
agent_extensions = ["extensions/example.ts"]
mcp_servers = "mcp/servers.json"
hooks = ["hooks/hooks.json"]
assets = "assets"

[release]
url = "https://plugins.example/example-bundle.zip"
artifact_kind = "zip-bundle"
sha256 = "<64-character-hex-digest>"
signature = "<ed25519-signature-base64>"
signing_key_id = "release-2026"
signature_scheme = "manifest-envelope-v1"
```

`artifact_kind` is covered by the release-envelope signature. A ZIP bundle may
omit `entry_component` when it declares another component. Mitsuro verifies the
signature and compressed-artifact digest before extraction, writes the signed
manifest itself, and then extracts into a fresh manager-owned transaction.
Archive paths must be enclosed relative paths and may not contain backslashes,
drive/alternate-stream colons, duplicates, or `plugin.toml`. Entries are
create-new: archive content cannot replace the signed manifest or another
entry. Unix symlinks and special entry types such as devices, FIFOs, and
sockets fail closed. Windows ZIPs without Unix mode metadata remain portable:
Mitsuro uses the directory marker and always creates other entries as new regular
files. ZIP64 and multi-disk archives are rejected before central-directory
allocation because their extended counts are unnecessary under these limits.
The archive is limited to the same 100,000 materialized filesystem entries,
512 MiB aggregate uncompressed data, and 64 MiB per file as unsigned package
snapshots. The entry and byte totals include implicit directories, the snapshot
root, and the authenticated manifest; actual decompressed bytes are counted as
well as ZIP header declarations. Every declared component is containment-checked and
must exist before the transaction is published. This signed bundle path is
fully supported on non-Unix platforms even though unsigned local/npm snapshots
fail closed there.

`signature_scheme` is mandatory and is part of the signed envelope. Mitsuro
does not guess whether a legacy signature covered only artifact bytes: a legacy
publisher must add the scheme and re-sign the manifest. Unknown schemes fail
closed so future protocols cannot be confused with this envelope format.
npm and local package installs are explicitly labeled unsigned; selecting a
catalog entry does not turn an npm package into a cryptographically signed
artifact.

Signing keys are bound to publisher identities so one allowlisted publisher
cannot name another publisher's trusted key:

```text
/plugins allow-publisher example.publisher
/plugins add-key release-2026 <public-key-base64> example.publisher
```

Key IDs are immutable; publisher key rotation uses a new ID instead of silently
reassigning existing trusted key material.
Old trust files may contain key material without a publisher binding. Mitsuro
never infers that binding from the allowlist. Re-run `/plugins add-key` with the
publisher and original key material, or use
`PluginManager::bind_existing_trusted_key_to_publisher` to explicitly bind an
existing stored key.
Publishers can generate the exact domain-separated bytes with the exported
`mitsuro_core::plugins::plugin_release_signing_payload` helper; Mitsuro verifies
that envelope before it downloads the referenced artifact, then verifies the
artifact digest before publication.

The installed immutable manifest retains the scheme, signature, digest, key ID,
and canonical manifest source needed for later activation-time revalidation.
`source_trust = signed-publisher` records install-time provenance; it is not by
itself proof that the currently stored bytes were revalidated during activation,
so API responses do not claim `cryptographically_verified` from that enum alone.

## Install

Local package directory:

```text
/plugins install ./examples/plugins/native-rust
```

npm package:

```text
/plugins install npm:@mitsuro/example-plugin
/plugins install npm:@mitsuro/example-plugin@1.2.3
```

Mitsuro never executes directly from a mutable source directory. It stages a
manager-owned snapshot, validates every manifest and component, atomically
publishes the snapshot, and finally atomically swaps `plugins.lock`. A failed
lock write rolls the snapshot back; `/plugins reconcile` removes snapshots left
by an interrupted process. Mutations are serialized across Mitsuro processes by
a bounded OS advisory lock on one stable, no-follow lock file; descriptor close
releases it after normal exit or a crash without deleting a successor's lock.

Unsigned local and npm package installation currently requires Unix. Windows
and other non-Unix builds reject those requests before creating a staging
transaction because Rust's stable filesystem API does not expose the complete
combination of no-follow opens, stable directory/file identity, and hard-link
counts used by the immutable snapshot proof. Mitsuro does not silently install a
weaker snapshot on those platforms. Signed single-component and signed ZIP
bundle manifests use the separate authenticated-artifact path and remain
available with full multi-resource distribution support.

npm lifecycle scripts and package build scripts execute arbitrary code, so they
are blocked by default. Packages must normally publish built artifacts. An
explicit, auditable opt-in is available when reviewing a trusted package:

```text
/plugins install ./path/to/package --allow-scripts
```

The opt-in is persisted with the lock entry and reused for updates. Command
stdout and stderr are drained continuously while only their recent bounded
tails are retained. Commands have a ten-minute timeout; expiry terminates the
complete process tree before the staged transaction is discarded.

While npm install or an explicitly approved build is running, Mitsuro also
rescans the complete staging root against the entry, aggregate-byte, and
per-file limits. A live violation terminates the same process tree and aborts
the transaction; a strict final scan remains mandatory before publication.
This polling check bounds normal package growth and catches abusive writers
quickly, but it is not a kernel filesystem quota, and explicit script consent
still grants code the host account's authority outside the staging directory.

Local and npm installs are bounded before publication. Mitsuro walks the complete
staging tree in deterministic filename order and permits at most 100,000
filesystem entries (including the snapshot root and directories), 512 MiB of
special files such as sockets, devices, and FIFOs are rejected rather than
followed or copied. Regular files with multiple hard links are rejected so a
mutable alias cannot change a reviewed snapshot. Local copies open each source
file with no-follow semantics and recheck file and parent directory identity
before and after copying. The plugin-manager root,
`.staging`, `.managed`, and `active` boundaries must remain real directories; a
symlink or non-directory fails closed before transaction enumeration or
mutation. npm installs use `--no-bin-links`, ensuring generated `.bin` links do
not enter a published snapshot. This intentionally means packages whose
install or build depends on command shims from `node_modules/.bin` are not
compatible with the npm build path; those packages must publish prebuilt
artifacts or be prepared locally before installation. The complete staged tree
is checked again immediately before publication so an explicitly allowed build
cannot add an over-quota or unsupported entry.

## Lifecycle, updates, and permissions

```text
/plugins pin <plugin-id>
/plugins unpin <plugin-id>
/plugins update [plugin-id|all] [--include-pinned]
/plugins uninstall <plugin-id>
/plugins reconcile [--update]
```

Exact npm versions, signed manifests, and local packages are pinned by default;
unpinned npm specs are eligible for normal updates. Updating a multi-plugin
package replaces the source as one snapshot, including removing components the
new package no longer declares. Uninstall removes grants and lock state first,
then deletes a manager-owned snapshot only after confirming that no sibling
plugin still references it.

Requested permissions are declarations, not grants. Review them and persist a
grant explicitly:

```text
/plugins permissions <plugin-id>
/plugins grant <plugin-id> all
/plugins grant <plugin-id> fs-read,network
/plugins revoke-permissions <plugin-id>
```

ID-based management flows use `PluginManager::ensure_plugin_permission`.
Runtime hosts that already hold an `InstalledPlugin` must instead use
`ensure_installed_plugin_permission` (or
`permission_status_for_installed` when evaluating the complete requested set),
so a concurrent same-ID replacement cannot authorize a stale descriptor.
Missing grants, legacy grants without a reviewed request snapshot, undeclared
access, grants made against an older requested set, and grants belonging to a
different publisher or source all fail closed.
Agent extensions and hooks must request `process`; MCP fragments must request at
least `process` or `network`. Hosts still require a current explicit grant before
activating those executable or external capabilities.

`process` is the trust boundary for native, JavaScript, TypeScript, and shell
components: granting it permits arbitrary local code with the current user's OS
authority. The remaining permission bits govern host-mediated capabilities and
make the requested authority reviewable, but they cannot sandbox a trusted Bun
worker or native library after `process` is granted. Installable
`runtime = "wasm"` entries do not require a process grant because they currently
present a managed descriptor without executing plugin code. Executable
Wasmtime isolation belongs to Mitsuro's separate Zed-compatible editor/language
host and is not a drop-in package TUI or agent-extension runtime.

Native and JS manifests with an `entry_component` are rejected unless they
declare `requested_permissions.process = true`, including signed release
manifests. Declaration alone does not load code: the TUI host resolves the
current, identity-bound grant before every catalog refresh and removes an
active native or JS component when that grant is absent or revoked. WASM entry
components do not require a process declaration or grant.

## Declarative package hooks

Entries under `hooks` are JSON or TOML command-hook configuration files. They
accept Mitsuro's flat hook format and the Codex/Claude-style event map. For
example:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^(bash|write)$",
        "hooks": [
          {
            "type": "command",
            "command": "./hooks/check-policy.sh",
            "timeout": 15
          }
        ]
      }
    ]
  }
}
```

`PreToolUse` commands can block by exiting with status 2 and placing the reason
on stderr. `PostToolUse` commands observe completed calls. Matchers are regular
expressions, hook input is JSON on stdin, commands run from the immutable
package root, stderr is bounded, and a timeout kills the complete process tree.
Invalid replacement input clears all ephemeral package hooks instead of leaving
stale executable policy active.

Only declarative `.json` and `.toml` files are accepted under `hooks`.
Executable JavaScript or TypeScript modules belong in `agent_extensions`, where
they use the governed persistent worker API described in
[Agent Extensions](agent-extensions.md).

## Plugin directory / catalog

The `/plugins` popup includes installed plugins plus an official, searchable catalog seeded from `docs/extensions/catalog.json`. The new `apps/website` Svelte/Bun site also publishes the same catalog at `/plugin-catalog.json` for the future `mitsuro.dev` relaunch. Press `/` in the popup to search; press `Enter` on an installed plugin to enable/disable it or on a catalog plugin to install its package reference.

Additional catalogs can be hosted as static JSON or TOML files locally or
behind HTTPS. Plain HTTP and redirects that downgrade from HTTPS are rejected:

```json
{
  "version": 1,
  "plugins": [
    {
      "id": "example",
      "name": "Example",
      "version": "1.0.0",
      "publisher": "example.publisher",
      "package": "npm:@mitsuro/example-plugin",
      "runtime": "native",
      "description": "Example searchable plugin listing",
      "tags": ["example"],
      "official": false
    }
  ]
}
```

Remote manifest, artifact, and catalog reads have explicit connection/header,
overall-request, and body-idle deadlines (20, 90, and 15 seconds respectively),
plus strict byte limits. Local manifests and artifacts are opened once and read
through a `max + 1` bounded handle, closing metadata/read races that could
otherwise bypass the declared size cap.

Only the built-in catalog can confer the `official` label. Mitsuro clears that
flag on entries loaded from configured third-party catalogs.

Register a catalog source with:

```text
/plugins add-source https://example.com/mitsuro-plugin-catalog.json example
/plugins remove-source example
/plugins catalog
```

## Runtimes

### native

Native plugins are dynamic libraries loaded through Mitsuro's C ABI. They are unsafe by design and are equivalent to executing arbitrary local code.

Rules:

- Export `mitsuro_plugin_entry`.
- Return a `MitsuroNativePluginV1` function table.
- Do not expose Rust trait objects across the dylib boundary.
- Keep persistent application state in the Mitsuro host when hot reload must preserve it.
- Treat plugin `Drop`/reload as shell lifecycle, not necessarily runtime shutdown.

Native reload uses a shadow copy of the entry dylib in `.mitsuro-shadow/` under the package/install root. This lets a source dylib be rebuilt while the old loaded copy remains mapped by the OS.
The TUI enumerates plugins from inert descriptors and instantiates only the
selected component. When a native host is dropped, it releases the library
handle and deletes its shadow copy.

### wasm

Package manifests accept `runtime = "wasm"`, but the installable TUI component
currently presents a managed descriptor and does not execute package code.
Mitsuro's executable Zed-compatible WASM extension host is loaded from the
global extension root and exposes a separate editor/language ABI. Bundle-only
packages should omit `entry_component` and contribute `agent_extensions`,
`skills`, hooks, or MCP instead.

### js

`runtime = "js"` runs JavaScript and TypeScript entry files through edon/libnode: load libnode dynamically, evaluate JS/TS through edon, and keep npm as the package boundary. Mitsuro looks for libnode at `MITSURO_LIBNODE` first, then `EDON_LIBNODE_PATH`.

JS/TS plugins register a small text-mode TUI object:

```ts
(globalThis as any).mitsuro.registerPlugin({
  tick() {},
  onActivate() {},
  onDeactivate() {},
  renderText() {
    return ["Hello from TypeScript"];
  }
});
```

Mitsuro evaluates `.ts`, `.tsx`, `.mts`, and `.cts` entries with edon's TypeScript evaluator and `.js` entries with the CommonJS evaluator. This is intentionally small for the first pass: text rendering and lifecycle hooks are supported; richer host callbacks/input APIs can be added once the runtime contract stabilizes.

## Reload

```text
/plugins reload <plugin-id>
```

For an active native plugin, reload drops the old plugin shell, shadow-copies the current entry dylib, loads the fresh copy, and recreates the plugin instance.

JS plugins are reinstantiated through the edon/libnode host. Install, update,
enable, disable, permission, and uninstall lifecycle changes rebuild the shared
skill, agent-extension, hook, and MCP contribution snapshot. `/plugins reload`
recreates the selected active native or JavaScript shell; a same-version
bundle-only descriptor is otherwise unchanged. Installable WASM TUI entries
remain managed descriptors while Zed-compatible WASM extensions use the
dedicated host.

## Example

See:

- `examples/plugins/native-rust/` for a minimal Rust cdylib plugin package.
- `examples/plugins/js-ts/` for a minimal edon/libnode TypeScript plugin package.
