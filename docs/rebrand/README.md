# Mitsuro v1 rebrand

Mitsuro is the product name. Honeycomb Technologies remains the company name. The v1 visual system combines a Graphite Wax foundation with restrained Obsidian Brass and mineral-violet accents.

The final product mark is one exact rounded cell rotated point-up. Hive uses three outlined cells. The ten-cell Honeycomb Technologies mark remains a corporate mark and is not repeated throughout the product UI.

Source-of-truth artwork lives in `assets/branding/mitsuro/`. Launcher and native splash PNGs in `apps/mobile/assets/` are renders of those sources. The splash Lottie is vector-only and traces all six cell sides simultaneously.

Current crate names, database fields, API routes, URL schemes, bundle IDs, and
release artifacts use Mitsuro and Hive. Prior identifiers are confined to the
tested migration readers and transition aliases listed by the canonical-name
audit.

The canonical GitHub repository is `honeycomb-Technologies/Mitsuro`, the App
Store Connect product name is Mitsuro, and mobile launch URLs use `mitsuro://`.

Windows archive installation changes binaries only. A machine with the prior
state root must keep every server and Hive generation stopped while
`mitsuro migrate-identity --confirm-offline` performs the one-time state
cutover; normal startup comes only after that command succeeds. The Linux shell
installer performs the same cutover under its procfs-proven offline service
handoff. macOS shell installation fails closed and prints the exact staged
migration command to run manually before retrying. Homebrew and AUR install
binaries or units only and require manual offline migration before first
startup when previous state exists.
