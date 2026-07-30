# Mitsuro v1 surface audit

This is the release boundary for the Mitsuro rebrand. It distinguishes product presentation from compatibility migrations so v1 can look and read as Mitsuro without silently breaking existing installations.

## Final product language

| Concept | Public name | Notes |
| --- | --- | --- |
| Product | Mitsuro | Lowercase outlined wordmark in artwork; title case in prose |
| Interactive assistant | Agent | Replaces mascot/personality naming |
| Persistent autonomous mode | Hive | “The hive is always alive.” |
| Delegated worker | Hive Agent | Deterministic numbered identities rather than sea-creature names |
| Company | Honeycomb Technologies | Corporate identity, not a repeated in-product motif |
| Autonomous activity accent | Pulse | Mineral-violet motion with restrained brass highlights |

## Completed v1 presentation surfaces

- Canonical point-up rounded-cell SVG family, lowercase wordmark, horizontal lockup, three-cell Hive icon, concave cast-metal launcher master, adaptive/tinted variants, and source color tokens.
- One-shot six-side simultaneous-trace Lottie splash animation.
- Expo display identity, launcher/splash/notification assets, onboarding, navigation, empty states, settings, sheets, reports, widgets, Live Activity, composer, model popover, markdown/code surfaces, run screens, and Hive action copy.
- Shared Graphite/Wax/Violet/Brass theme tokens for mobile, web, and desktop-hosted web.
- Native Skia and web SVG running-line treatments using the same restrained palette.
- Production removal of the visual prototype route and unreferenced crab/shark assets.
- Marketing website wordmark, favicon, metadata, navigation, product copy, palette, and plugin presentation.
- Tauri product name, window title, publisher, package descriptions, launcher icons, and shell copy.
- CLI and server public descriptions, startup copy, OAuth pages, recovery notices, notifications, user agents, Agent prompt identity, Hive command/help text, and default Hive session title.
- Installer output, Homebrew/AUR descriptions, systemd descriptions, Cargo package descriptions, active documentation prose, and root README.
- GitHub repository identity and active repository, installer, package-manager, website, native-module, and updater links.
- App Store Connect product name, installed iOS product name, and canonical `mitsuro://` deep-link launch URLs.
- A runnable rebrand audit that rejects retired mascot assets, slogans, colors, and accidental TUI edits.

## Intentionally deferred

The entire Ratatui surface under `crates/krusty-cli/src/tui/` is excluded. Its ASCII art, mascot animation, theme registry, onboarding, block styling, and terminal-specific interaction design remain legacy until the planned ground-up Mitsuro TUI rebuild. The current behavior is documented honestly instead of partially reskinning it.

## Compatibility identifiers retained for v1

These are not missed branding. They are stable contracts whose rename needs a separately designed migration with aliases, data movement, rollback, and release coordination.

| Area | Retained identifiers |
| --- | --- |
| Executables and crates | `krusty`, `krusty-mako`, `krusty-*` crates |
| Package namespaces | `@krusty/api`, `@krusty/state`, `@krusty/ui` |
| Server routes and schemas | `/api/mako/*`, `session_type: "mako"`, `Mako*` wire/storage types |
| Local state | `~/.krusty`, `krusty.db`, existing preference/storage keys |
| Services and IPC | `krusty-mako.service`, `krusty-mako.socket`, `mako.sock` |
| Links and platform IDs | legacy `krusty://` alias, Expo slug, existing iOS/Android/Tauri bundle identifiers |
| Native/source symbols | `KrustyClient`, `KrustyDiagnostics`, `MakoWidget`, and other compiled symbols |
| Distribution contracts | existing package, crate, executable, and formula names |
| Historical evidence | `docs/archive/` and versioned migration names |

User-facing labels around those contracts must say Mitsuro, Agent, or Hive. Documentation may show the compatibility spelling inside code formatting when that is the exact command, route, file, symbol, or stored value.

## Compatibility migration progress

- The canonical repository is `honeycomb-Technologies/Mitsuro`; GitHub redirects the previous repository URL for compatibility.
- `mitsuro://` is the canonical mobile deep-link scheme. `krusty://` remains registered and accepted so installed widgets, Live Activities, saved links, and older servers continue to work.
- The App Store Connect product name and the shipped iOS display/product name are Mitsuro.

The remaining compatibility migration may introduce new executable, route, package, state-directory, Expo-slug, and native-module identifiers. It must preserve old aliases long enough for desktop, mobile, server, package managers, systemd, saved credentials, deep links, and existing databases to move together.

Existing Apple and Android application identifiers are installation identities rather than display branding. The current iOS app record cannot change its bundle ID after a build has been uploaded; replacing `io.krusty.mobile` would require a new app record and break the existing update/TestFlight lineage. It therefore remains an opaque compatibility key for the current product.
