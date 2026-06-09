# Game Boy Color TUI Plugin Implementation Context

Task scope: convert Krusty's existing RetroArch/libretro TUI plugin into a Game Boy Color branded plugin that can ship/prefer a bundled Gambatte/SameBoy libretro core and keep emulator runtime state host-owned across plugin shell reloads. No source files were modified while gathering this context.

## Current architecture

### Built-in TUI plugin registry

- `crates/krusty-cli/src/tui/plugins/mod.rs:18-25` conditionally exposes `gamepad`, `libretro`, and `retroarch` only on Unix. Non-Unix gets a no-op `GamepadHandler` but not libretro/retroarch.
- `Plugin` trait (`mod.rs:125-168`) is the in-process shell contract: `id`, `name`, `display_name`, `render_mode`, `render`, `render_frame`, `handle_event`, `tick`, lifecycle hooks, and `as_any_mut` for downcasts.
- Built-ins are recreated on demand:
  - `builtin_plugins()` currently inserts `RetroArchPlugin::new()` first (`mod.rs:171-179`).
  - `get_plugin_by_id("retroarch")` returns a new `RetroArchPlugin` (`mod.rs:182-187`).
- Installed plugin descriptors are separate (`InstalledPluginDescriptor` at `mod.rs:34-67`) and are read from the core plugin manager catalog; built-in RetroArch is not managed by `PluginManager`.

### Plugin window lifecycle and reload behavior

- `PluginWindowState` owns the active `Box<dyn Plugin>` (`components/plugin_window.rs:52-84`). This is currently the host shell state and the emulator runtime state for RetroArch, because the RetroArch core lives inside the plugin object.
- `toggle()` loads the preferred active plugin or the first built-in if none is active (`plugin_window.rs:110-138`). Hiding the plugin window clears graphics but does **not** drop the active plugin.
- `set_plugin()` clears graphics, calls `old.on_deactivate()`, activates the new plugin, and replaces `active_plugin` (`plugin_window.rs:151-170`). Replacing a RetroArch plugin drops the old boxed plugin.
- `tick()` polls gamepads, then has a hard-coded RetroArch downcast guarded by `plugin.id() == "retroarch"` (`plugin_window.rs:245-264`), then calls the active plugin's `tick()` (`plugin_window.rs:266-273`). This must change for a renamed/branded plugin.
- Rendering checks the active plugin's `render_mode()` and either calls `render()` or `render_frame()`; Kitty graphics support gates frame rendering (`plugin_window.rs:346-470`).
- `/plugins reload <plugin-id>` is only for installable plugins. It calls `PluginManager::reload_plugin`, and if the active plugin ID matches, it rebuilds the shell with `get_plugin_by_id()` and `set_plugin()` (`handlers/commands/plugins/subcommands.rs:111-129`). Because built-in `retroarch` is not in the installed plugin lockfile, `/plugins reload retroarch` currently fails in `PluginManager::reload_plugin()` (`krusty-core/src/plugins/manager/mod.rs:107-113`).
- Catalog refresh updates installed plugin descriptors and can clear the active plugin if the ID disappears (`handlers/commands/plugins/catalog.rs:4-65`). Built-ins are not catalog-managed.

### RetroArch/libretro implementation

- `crates/krusty-cli/src/tui/plugins/libretro.rs` defines raw libretro FFI. `LibRetroCore` owns a `libloading::Library` and function pointers (`libretro.rs:164-195`), loads a dynamic library (`libretro.rs:197-207`), resolves required symbols (`libretro.rs:217-267`), and validates API version 1 (`libretro.rs:270-276`). Current loading is Unix-only (`libretro.rs:202-283`).
- `RetroArchPlugin` owns all runtime state:
  - `core: Option<LibRetroCore>`, `rom_path`, `core_name`, `shared_state`, `running`, `av_info`, `error`, menu state, core/ROM lists, current ROM dir, etc. (`retroarch.rs:419-460`).
  - `SharedState` carries the frame buffer, pixel format, frame count, button press frames, and scratch buffer (`retroarch.rs:113-145`).
- Libretro callbacks are global because the C API has no Rust context:
  - `SHARED_STATE: Mutex<Option<Arc<Mutex<SharedState>>>>` (`retroarch.rs:147-148`).
  - `set_shared_state`, `clear_shared_state_if_owner`, `with_shared_state` (`retroarch.rs:150-181`).
  - This already prevents a dropped plugin from clearing a newer owner's state, but the core itself is still plugin-owned and dropped on shell replacement.
- Environment callbacks use static paths under `~/.config/krusty/retroarch/{system,saves}` (`retroarch.rs:335-348`) and `KrustyDirs` uses `~/.config/krusty/retroarch/{system,saves,states,roms}` (`retroarch.rs:468-498`). This is inconsistent with core path utilities/docs that use `~/.krusty` for plugins and app state.
- Core loading (`retroarch.rs:531-567`) always calls `self.unload()` first, loads the dynamic library, sets libretro callbacks, initializes, stores core name and core object.
- ROM loading (`retroarch.rs:569-624`) reads the ROM into memory, calls `retro_load_game`, gets AV info, sets controller port 0 to joypad, marks `running = true`, and loads SRAM.
- `unload()` unloads ROM/core, deinitializes, clears global shared state if owner, and resets runtime fields (`retroarch.rs:626-640`). `Drop for RetroArchPlugin` calls `self.unload()` (`retroarch.rs:1622-1625`), so any shell replacement loses runtime state today.
- `scan_cores()` only looks at `/usr/lib/libretro` and only accepts `.so` (`retroarch.rs:686-699`). This is Linux-centric despite Unix cfg; macOS `.dylib` would not be found.
- ROM filtering already recognizes Gambatte as GB/GBC (`retroarch.rs:743-785`), but default filtering accepts many non-GB systems.
- Menu currently has `Main -> CoreBrowser -> RomBrowser`, so users manually pick any system core (`retroarch.rs:931-1063`). Main menu renders `Load Game` and `Settings` (`retroarch.rs:1142-1167`); core browser renders all discovered cores.
- Branding is currently hard-coded as RetroArch:
  - `name() -> "RetroArch"` (`retroarch.rs:1418-1420`).
  - `display_name()` uses `RetroArch` strings (`retroarch.rs:1423-1435`).
  - `render_menu()` title for main menu is `"RetroArch"` (`retroarch.rs:1090-ish; see title match before `retroarch.rs:1110`).
  - Logs and env vars use `KRUSTY_RETROARCH_CORE` / `KRUSTY_RETROARCH_ROM` (`retroarch.rs:1583-1608`).
- Save state/SRAM paths are ROM-stem based under the static dirs (`retroarch.rs:807-929`). Battery save is read/written through `RETRO_MEMORY_SAVE_RAM`.

### Installable plugin manager/docs

- `PluginManager` manages signed installable plugins under `~/.krusty/plugins` with `installed`, `active`, `state`, `index`, `trust`, and `plugins.lock` roots (`krusty-core/src/plugins/manager/mod.rs:21-67`; docs `docs/extensions/mcp-and-plugins.md:102-118`).
- Manifest v1 describes a single `entry_component` artifact plus render capabilities and permissions (`krusty-core/src/plugins/types.rs:44-89`; docs `mcp-and-plugins.md:82-90`).
- Installation verifies publisher allowlist and artifact SHA/signature before writing a single artifact to the install dir (`manager/install.rs:20-62`, `64-90`; docs `mcp-and-plugins.md:92-100`).
- Current `ManagedPlugin` is only a lightweight placeholder, not a WASM/runtime execution host. It renders static text or demo frames and handles `r` locally by resetting animation (`crates/krusty-cli/src/tui/plugins/managed.rs`).
- Therefore, the Game Boy Color work should target the built-in TUI plugin path unless a larger installable-plugin runtime is intentionally introduced.

## External facts relevant to bundled cores

- Gambatte libretro is GPL-2.0 (`libretro/gambatte-libretro` GitHub/COPYING, web search 2026-06-08). Bundling it may impose GPL distribution obligations for the shipped artifact/package.
- SameBoy is MIT licensed in upstream/libretro repositories (web search 2026-06-08). It is a safer default for bundled distribution from a licensing standpoint.
- Libretro docs confirm cores use frontend-provided system/save directories and `RETRO_MEMORY_SAVE_RAM` for cartridge battery-backed save RAM; Krusty's existing implementation already provides `GET_SYSTEM_DIRECTORY`, `GET_SAVE_DIRECTORY`, save-state serialization, and SRAM memory handling.

## Minimal safe implementation path

1. **Keep this as a built-in TUI plugin, not an installable managed plugin.** The managed plugin system currently has no real executable plugin host; trying to route libretro through `PluginManager` would create a much larger feature.
2. **Introduce a Game Boy Color-branded plugin type while preserving libretro internals.** Practical options:
   - Rename/wrap `RetroArchPlugin` to `GameBoyColorPlugin` in a new `gameboy.rs` or refactor `retroarch.rs` into a generic libretro host plus GBC shell.
   - Register the built-in as `id() == "gameboy-color"`, `name() == "Game Boy Color"`.
   - Consider a temporary alias for old preference ID `retroarch`, but avoid exposing both if only one libretro global callback host is supported.
3. **Move emulator runtime out of the plugin shell.** This is the key requirement for host-owned runtime state across shell reloads.
   - Add a host-owned singleton/registry, e.g. `static GAMEBOY_RUNTIME: LazyLock<Arc<Mutex<GameBoyRuntime>>>` or a `PluginHostRuntime` stored in `PluginWindowState`.
   - Runtime owns: `core`, `rom_path`, `core_name`, `shared_state`, `running`, `av_info`, save/SRAM methods, explicit unload/exit.
   - Plugin shell owns UI-only state: menu cursor/scroll/current ROM dir/error display if desired. If error state should survive reload, put it in runtime too.
   - `Drop` for the shell must not call `unload()`. Runtime should unload only on explicit `Exit ROM`, process exit, or a deliberate reset command. Otherwise `/plugins reload`-style shell replacement will still lose state.
   - `on_activate()` should call `set_shared_state(runtime.shared_state.clone())` so libretro callbacks target the host-owned state after shell recreation.
4. **Prefer a bundled SameBoy/Gambatte core via an explicit resolver.** Replace `scan_cores()` with a GBC core resolver:
   - Candidate order: env override (`KRUSTY_GAMEBOY_CORE` and possibly legacy `KRUSTY_RETROARCH_CORE`), extracted bundled SameBoy, extracted bundled Gambatte (if approved), user-installed cores under `~/.krusty/gameboy-color/cores`, system `/usr/lib/libretro/{sameboy,gambatte}_libretro.so`.
   - Keep an actionable fallback message if no GBC core exists.
   - Support target-specific dynamic library extension/name (`.so` Linux, `.dylib` macOS; current code only scans `.so`). Windows is currently out of scope because libretro module is `cfg(unix)`.
   - If bundling through `include_bytes!`, write the core to a stable host-owned runtime path before `libloading::Library::new`; libloading needs a real file path. Include checksum/version in the extracted filename or sidecar to avoid stale binaries after updates.
   - If bundling as release/package assets instead, wire package/install scripts to place cores under `~/.krusty/gameboy-color/cores` or an app resource dir and teach the resolver where to find them.
5. **Restrict UI/ROM behavior to Game Boy/Game Boy Color.**
   - Main menu can go directly to ROM browser after resolving the preferred core; no generic core browser needed for a branded plugin.
   - `is_rom_file()` should accept only `.gb`/`.gbc` for this plugin.
   - Change copy: `Load Cartridge`, `Select Game Boy ROM`, `Game Boy Color`, `Core: SameBoy/Gambatte`.
6. **Update plugin window gamepad bridge.** Replace the hard-coded `plugin.id() == "retroarch"` / downcast to `RetroArchPlugin` (`plugin_window.rs:250-257`) with either:
   - `plugin.id() == "gameboy-color"` and downcast to `GameBoyColorPlugin`, or
   - better, extend the `Plugin` trait with an optional `handle_gamepad_button(button_id)` default no-op to remove concrete downcasting.
7. **Use centralized app paths.** Prefer `crate::paths::config_dir().join("gameboy-color")` or `krusty_core::paths::config_dir()` over the current `~/.config/krusty/retroarch`. Keep `system`, `saves`, `states`, `roms`, `cores` directories under that base. Consider migration or reading legacy save dirs if existing users matter.

## Files/functions likely to change

- `crates/krusty-cli/src/tui/plugins/mod.rs`
  - Add `gameboy` module/export; register `GameBoyColorPlugin` in `builtin_plugins()`; change `get_plugin_by_id` handling from `retroarch` to `gameboy-color` (optional legacy alias).
- `crates/krusty-cli/src/tui/plugins/retroarch.rs` and/or new `crates/krusty-cli/src/tui/plugins/gameboy.rs`
  - Refactor runtime ownership away from plugin shell.
  - Rename branding strings, IDs, env vars, paths.
  - Replace generic core browser/scan with GBC resolver and bundled-core extraction/preference.
  - Restrict ROM extensions.
  - Preserve/save SRAM and save-state behavior.
- `crates/krusty-cli/src/tui/components/plugin_window.rs`
  - Update hard-coded RetroArch downcast or replace with trait-based gamepad event hook.
  - If host-owned runtime lives in `PluginWindowState`, add field/init here.
- `crates/krusty-cli/Cargo.toml`
  - Only if adding helper dependencies or features for asset extraction/checksums. Note this file is already staged-modified in the dirty worktree.
- Packaging/release asset files (not inspected deeply here)
  - Needed if truly shipping native SameBoy/Gambatte cores rather than only preferring user-installed system cores. Search/update release workflows, AUR, desktop packaging as needed.
- Docs/help strings
  - `docs/extensions/mcp-and-plugins.md` if describing built-in GBC plugin/core paths.
  - Help/autocomplete only if adding commands/env vars.

## Likely tests and validation

Targeted tests to add:

- Unit tests in the refactored GBC module for:
  - core candidate ordering and env override precedence;
  - target-specific libretro file extension selection;
  - `.gb`/`.gbc` ROM filtering and rejection of non-GB extensions;
  - shell reload persistence: create plugin shell A, mutate/load mocked runtime state, drop/replace with shell B, assert runtime `Arc`/state persists. This may require factoring runtime behind a testable trait or test-only fake `LibRetroCore` because real dynamic core loading is integration-level.
- Unit tests in `plugin_window.rs` if replacing downcast with a trait hook: assert gamepad button dispatch reaches a fake plugin without ID/type coupling.
- Existing plugin manager tests are less relevant unless packaging through installable plugins; current manager tests cover signed install/lockfile behavior.

Validation commands:

- `cargo test -p krusty` for TUI plugin unit tests.
- `cargo check -p krusty` after refactor.
- If touching core plugin manager: `cargo test -p krusty-core plugins::manager` and `cargo check --workspace`.
- Manual validation on a Kitty-compatible terminal: open plugin window (`Ctrl+P`), focus it, load a `.gbc` ROM, confirm frame rendering, keyboard/gamepad controls, pause/save/load state, hide/show window without stopping, and trigger any reload/shell replacement path to confirm game continues.
- Full repo validation per `AGENTS.md` remains `cargo check --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --all`, plus web export if frontend touched. Expect noise due dirty tree (see below).

## Implementation risks / constraints

- **Runtime ownership is the core risk.** Today `Drop for RetroArchPlugin` unloads the core. Any shell reload/replacement will still lose state until core/runtime ownership moves out of the plugin object.
- **Global libretro callbacks allow only one active callback target.** Existing `SHARED_STATE` is a singleton. A GBC-only built-in is fine; multiple concurrent libretro plugins would need a more careful host design or explicit mutual exclusion.
- **Do not hold a runtime mutex while invoking callbacks if it can deadlock.** `retro_run()` calls back into `video_refresh`, which locks `SHARED_STATE`. Keep runtime and shared frame locks separate and avoid lock inversion.
- **Bundled native core packaging is platform-specific.** Need Linux `.so` and macOS `.dylib` at minimum for current `cfg(unix)`. Existing scan is Linux-only. Windows is not supported by current libretro module.
- **Licensing decision needed before bundling Gambatte.** SameBoy (MIT) is safer as default; Gambatte libretro is GPL-2.0. If bundling Gambatte, confirm project license/distribution obligations.
- **State path migration.** Existing saves are under `~/.config/krusty/retroarch`; new branded paths should probably live under `~/.krusty/gameboy-color`. Decide whether to migrate/read legacy saves/states.
- **Plugin manager reload path does not apply to built-ins.** If product wants `/plugins reload gameboy-color`, `PluginManager::reload_plugin` currently rejects non-installed built-ins. A separate built-in reload command or a plugin-window shell reset path may be needed; don't force this through installable plugin infrastructure without a larger design.

## Dirty worktree risks observed

- The worktree is very dirty overall (many modified/added files across CLI/core/server/apps). This can make workspace validation fail for unrelated reasons and increases conflict risk.
- Relevant paths checked:
  - `crates/krusty-cli/Cargo.toml` is staged-modified (version `0.6.2 -> 0.7.0`). If adding deps/features, preserve this user change.
  - `crates/krusty-cli/src/tui/app_builder.rs` is staged-modified in model registry code, not plugin setup. Early plugin-manager initialization lines are unchanged, but avoid overwriting this file unless necessary.
  - No dirty changes were reported under `crates/krusty-cli/src/tui/plugins/`, `crates/krusty-cli/src/tui/components/plugin_window.rs`, `crates/krusty-cli/src/tui/handlers/commands/plugins/`, `crates/krusty-core/src/plugins/`, or `docs/extensions/mcp-and-plugins.md` at the time of inspection.
