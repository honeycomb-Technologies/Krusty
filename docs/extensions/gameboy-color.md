# Game Boy Color Extension

Mitsuro ships a built-in **Game Boy Color** plugin (`gameboy-color`) backed by libretro cores. It runs user-provided `.gb` and `.gbc` ROMs, including Pokémon Yellow (`.gb`), without bundling ROMs or Nintendo BIOS files.

## Core discovery

The plugin prefers Game Boy-capable libretro cores in this order:

1. `KRUSTY_GAMEBOY_COLOR_CORE` (single core path)
2. `KRUSTY_RETROARCH_CORE` (legacy single core path)
3. `KRUSTY_GAMEBOY_COLOR_CORES` (path list of core directories)
4. `~/.config/krusty/gameboy-color/cores`
5. `~/.config/retroarch/cores`
6. Packaged app-relative core directories:
   - `gameboy-color/cores`
   - `cores/gameboy-color`
   - `../share/krusty/gameboy-color/cores`
   - `../lib/krusty/gameboy-color/cores`
7. `/usr/lib/libretro`
8. `/usr/local/lib/libretro`

Recognized core filenames/hints include `gambatte_libretro.so`, `sameboy_libretro.so`, `gearboy_libretro.so`, and `mgba_libretro.so`. Gambatte and SameBoy are preferred.

## Runtime data

Mitsuro creates and uses:

- ROMs: `~/.config/krusty/gameboy-color/roms`
- Per-core system data: `~/.config/krusty/gameboy-color/system`
- Battery saves: `~/.config/krusty/gameboy-color/saves`
- Save states: `~/.config/krusty/gameboy-color/states`

## Hot-reload boundary

The libretro core, loaded ROM, save data, current frame, and input timing live in a host-owned runtime. Plugin shell instances only own menu/render/input behavior, so replacing or hot-reloading the shell does not intentionally unload the emulator. Choosing **Exit ROM** explicitly saves SRAM and unloads the runtime.
