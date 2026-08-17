# Desktop Terminal Policy: Ghostty-first

Mitsuro Desktop prefers **Ghostty** as the real workstation terminal.

## Research snapshot
Ghostty is both:
- a native app (current best desktop UX)
- an embeddable library family (`libghostty` / `libghostty-vt`)

Today:
- full native embed SDK is still maturing
- `libghostty-vt` is real (C/Zig/WASM VT core)
- mobile candidate exists (`expo-libghostty`) but web is unsupported

Mitsuro Desktop ships Ghostty through both the native host launch and its embedded Ghostty WASM surface.

## Behavior
- Code plane utility tab: **Ghostty**
- Primary action: open Ghostty in active project directory
- Secondary: copy open command
- Tertiary: embedded fallback terminal

## Host bridge
Tauri command:

```rust
open_ghostty(directory?: string)
```

macOS launch:

```bash
open -na Ghostty --args --working-directory=<dir>
```

## Scope
- Desktop/native host: Ghostty-first
- Pure web preview: command + fallback only
- Mobile later: evaluate `expo-libghostty` against Mitsuro `/ws/terminal`
