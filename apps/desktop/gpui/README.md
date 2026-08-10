# Mitsuro GPUI Desktop

Experimental native desktop client built with GPUI and GPUI Component.

This directory contains the maintained source imported from the earlier standalone
prototype. The original local directory remains an unchanged forensic snapshot;
generated targets, self-review reports, reverse-engineering dumps, and duplicate
screenshots were intentionally not imported.

## Product boundary

The desktop must support two explicit transports through normalized product concepts:

- Mitsuro server over HTTP and SSE, using `mitsuro-client`.
- Codex app-server over managed stdio, with WebSocket support tracked separately.

Fixture mode is for deterministic development and tests. A generic fixture success is
not evidence that a product feature works. Production connections never seed, densify,
or fall back to fixture records: loading, empty, unsupported, and error are distinct UI
states. `MITSURO_SKIP_APPSERVER` now leaves an explicit backend-disabled error instead of
quietly entering fixture mode.

The selected transport, backend-qualified session, backend-scoped model, and
privacy-safe desktop preferences are persisted in
`~/.mitsuro/gpui-desktop-state.json` (override with `MITSURO_GPUI_STATE_PATH`). An
explicit `MITSURO_BACKEND` always takes precedence. No server token or provider
credential is written to this file.

Settings → Connections can switch between the Mitsuro HTTP/SSE client and a managed
Codex app-server stdio child without restarting GPUI. Switching disconnects the prior
transport, ignores stale bootstrap results, and restores the last selected session for
each backend. Connection errors remain errors; they do not silently enable fixtures.

## Current live surfaces

- Sessions, models, turns, files, skills, MCP servers, and extensions use the
  transport-neutral `ProductBackend` contract.
- Live terminal, file, account, environment, extension, Work, and Scheduled failures
  remain attached to their originating backend. They never retry against the fixture
  backend, and a backend switch clears the previous backend's projection immediately.
- An authenticated Ready Mitsuro or Codex backend sends a real turn by default.
  Fixture turns require an explicit fixture backend or fixture environment flag, and
  session/turn failures remain visible errors instead of replaying synthetic success.
- Mitsuro Work is a read-only projection of `/api/hive/current`; Hive dispatch and
  task mutations are deliberately unavailable in GPUI for now.
- Scheduled reads `/api/hive/schedules`; create, pause, resume, and delete are not
  exposed until their product interaction and approval semantics are designed.
- Terminal shows Mitsuro's `/api/processes` catalog read-only. Codex stdio retains
  interactive `process/*`; Mitsuro does not pretend its background-process API is a PTY.
- Atlas is an explicit system-browser bridge in the default build. It stores local URL
  history and opens real pages externally; it does not fabricate page content, import
  browser profiles, or claim access to browser-owned cookies and history.
- Secondary Settings actions without an implementation are non-interactive and labeled
  `Not wired` or `Unavailable`. Account, backend, and connection actions retain their
  separate live implementations.
- Account and Usage render protocol data only when the connected backend supplies a
  complete snapshot. Mitsuro HTTP shows an explicit unsupported state; it does not show
  sample identities, plans, credits, limits, or billing history.
- The composer exposes only implemented behavior: text entry, Send, Stop, and
  backend-scoped model cycling. Attachment, voice, project, and access stubs are not
  presented as controls.
- Long transcripts start with a 16-message tail and reveal earlier history in bounded
  pages. Reopened threads preserve structured reasoning, plans, commands, and file
  changes. The current 18-type Codex thread item surface is preserved across hydration
  and live updates: non-chat tool/search/image/collaboration/review lifecycle records
  render as real, cardless activity rows rather than being dropped or converted into
  assistant prose. Assistant Markdown, fenced code, visible errors, and bounded
  full-response expansion render while the composer remains pinned outside the
  transcript scroll.

## Parity status

The home shell, open transcript, Settings, and every product destination have been
reviewed in a live 940×1054 GPUI window against the reversed ChatGPT desktop reference.
Pull requests and Sites retain their navigation destinations but render
explicit capability states: neither backend exposes a typed API for those products, so
the native client does not show sample repositories, sample deployments, or inactive
create/review controls. Atlas/browser, the composer, live-turn failure handling, and
secondary Settings actions follow the same honest capability treatment. Desktop-only
Settings values are durable and explicitly distinguished from live server configuration.
The release candidate has passed the complete surface matrix and strict dual-provider
live acceptance. The production-data purity slice adds a source-level fixture gate and
fresh live captures for Work, Scheduled, Computer, Extensions, Settings, and Files on
both transports. Installation and deployment remain separate operator actions.

## Build

```bash
cargo check -p mitsuro-desktop-backend
cargo check -p mitsuro-gpui-desktop
cargo test -p mitsuro-desktop-backend
cargo test -p mitsuro-client
cargo test -p mitsuro-gpui-desktop --no-default-features
```

Run against the local Mitsuro server without authorizing provider turns:

```bash
MITSURO_BACKEND=mitsuro-http MITSURO_NO_LIVE_TURN=1 \
  cargo run -p mitsuro-gpui-desktop
```

Use `MITSURO_BACKEND=codex-stdio` for a managed Codex app-server child. A Ready,
authenticated backend sends a provider-backed turn when the user presses Send. Keep
`MITSURO_NO_LIVE_TURN=1` for read-only visual validation; use
`MITSURO_FORCE_FIXTURE=1` only for explicit fixture tests.

The Codex adapter negotiates experimental APIs because the desktop exposes process,
environment, realtime, and background-terminal protocol families. The reviewed
`codex-cli 0.147.0` contract is checked with:

```bash
scripts/gpui-codex-protocol-check.sh
```

The optional `browser-native` feature links Wry/WebKitGTK. The default build uses the
external-browser bridge only while native embedding remains incomplete.
