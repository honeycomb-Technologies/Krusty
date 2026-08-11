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

The selected transport, backend-qualified session, backend-scoped model, ordered
backend-scoped pinned-session ids, and privacy-safe desktop preferences are persisted in
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
- Pinned recents mirror the native Codex host contract: Pin/Unpin is desktop-local,
  ordered, and scoped to the exact Mitsuro or Codex backend session identity. The current
  Codex app-server contract does not own pin state. A saved pin only reorders a thread
  returned by the active backend; it cannot synthesize a row, and fixture/draft rows are
  not pinnable.
- The sidebar bell switches to the reference Activity catalog: authoritative in-flight
  interactions appear under Priority, followed by pinned work and real session update
  dates grouped as Today, Yesterday, or full weekday. Mitsuro RFC 3339 values
  are normalized at the HTTP adapter boundary; Codex epoch values flow through unchanged.
  Missing or invalid timestamps remain explicitly grouped as Earlier.
- Codex-surface Projects also follow the native-host contract. GPUI persists only a
  stable project name and canonical folder roots, then filters each backend's real
  session catalog by its authoritative `working_dir`. The same saved project works after
  switching between Mitsuro HTTP and Codex stdio; removing it keeps all server threads
  and files. Fixture mode cannot create or select projects.
- Codex plugin installation and removal use typed app-server methods and refresh the
  live marketplace afterward. Mitsuro and fixture extension inventories are read-only;
  the desktop never simulates a successful mutation. Marketplace search filters live
  plugin, skill, and MCP metadata, and section expansion reveals the exact hidden
  server records rather than fabricated marketplace totals.
- MCP rows requiring authentication can start the typed Codex OAuth flow, open the
  returned authorization URL in the system browser, wait for the matching completion
  notification, and refresh live connector status. Mitsuro remains read-only here.
- Settings → Connections can add real Codex HTTP or stdio MCP servers through typed
  `config/value/write` followed by `config/mcpServer/reload`. The form validates names,
  URLs, executable-only command fields, and JSON string-array arguments before I/O.
  Mitsuro does not expose the mutation, so the form is replaced by an unsupported note.
- Settings → Hooks reads the exact Codex `hooks/list` catalog for the active workspace,
  including handler/event/source/path, trust, managed, warning, and error state. Mitsuro
  and explicit fixture mode show an empty unsupported/fixture state, never sample hooks.
- Settings → Remote control reads the real Codex host identity and connection state,
  tracks `remoteControl/status/changed`, creates and checks real pairing codes, paginates
  the authorized-device catalog, and revokes access only after a second confirmation.
  Mitsuro HTTP and explicit fixture mode show honest unsupported/fixture states. The
  removed local “Computer use” toggles never changed either backend and are no longer
  presented as operational controls.
- Settings → Import detects real Claude Code and Cursor content through typed
  `externalAgentConfig/detect` calls, shows the exact returned migration groups for
  review, and requires an explicit confirmation before calling
  `externalAgentConfig/import`. Progress/completion notifications and
  `externalAgentConfig/import/readHistories` drive status and history. The current
  local app-server does not expose Claude Cowork as a distinct migration selector, so
  GPUI does not present a fake Cowork action. Mitsuro HTTP and explicit fixture mode
  show honest unsupported/fixture states.
- General → Experimental features lists only user-facing beta rows returned by
  `experimentalFeature/list`. Toggle changes use the same persistent
  `features.<name>` configuration path as the reference app through typed atomic
  `config/batchWrite`, then re-read effective state. The prior hard-coded Plugins and
  Request user input toggles were local decoration and have been removed. Mitsuro HTTP
  and fixture mode show explicit unsupported/fixture states.
- Live terminal, file, account, environment, extension, Work, and Scheduled failures
  remain attached to their originating backend. They never retry against the fixture
  backend, and a backend switch clears the previous backend's projection immediately.
- An authenticated Ready Mitsuro or Codex backend sends a real turn by default.
  Fixture turns require an explicit fixture backend or fixture environment flag, and
  session/turn failures remain visible errors instead of replaying synthetic success.
- The latest persisted Codex user message exposes the reference-style hover action and
  double-click inline editor. Send performs one real `thread/rollback`, replaces the UI
  from the returned thread, and resubmits the edited text with every retained local,
  remote, embedded, skill, and mention input. Cancel is local and non-destructive.
  Navigation/backend switching is held during the rollback/resubmit boundary. Mitsuro
  hides editing because its HTTP API has no destructive turn rollback.
- Mitsuro Work reads the authoritative `/api/hive/current` catalog and the selected
  session's `/api/hive/sessions/:id/status` detail. The native screen renders real task
  rows rather than counter-derived pseudo-plan items, prefers runtime state over stale
  agent state, and exposes typed dispatch, message, pause/resume, priority, crew, and
  confirmed cancellation controls. Each write carries a unique idempotency key and
  refreshes both the catalog and selected-session detail after success. Codex renders an
  explicit unsupported state because app-server has no Mitsuro Hive control plane.
- Scheduled reads `/api/hive/schedules` and exposes real Mitsuro pause, resume, and
  cancellation controls. Every mutation sends the schedule revision through `If-Match`
  plus a unique idempotency key, refreshes the authoritative catalog after success, and
  requires a second click before cancellation. The native editor creates and replaces
  schedules with the full server contract: all recurrence variants, timezone/DST,
  workspace/model/crew identity, priority, misfire, overlap, and retry policy.
- Terminal shows Mitsuro's `/api/processes` catalog read-only. Codex stdio launches
  standalone commands through the current sandboxed `command/exec*` family, including
  streamed stdout/stderr, stdin, resize, termination, and the process-exit response.
  The older `process/*` adapter remains only as an explicit compatibility/fixture path;
  Mitsuro does not pretend its background-process API is an interactive PTY.
- Computer can register a real Codex remote exec-server with `environment/add`, then
  retains the submitted id and URL for the app session and probes typed status/info.
  The form is hidden for Mitsuro because its HTTP API has no equivalent mutation, and
  no live registration is performed by automated acceptance.
- Atlas is an explicit system-browser bridge in the default build. It stores local URL
  history and opens real pages externally; it does not fabricate page content, import
  browser profiles, or claim access to browser-owned cookies and history.
- Secondary Settings actions without an implementation are non-interactive and labeled
  `Not wired` or `Unavailable`. Account, backend, and connection actions retain their
  separate live implementations.
- Generic Settings controls are fail-closed rather than decorative. Send shortcut,
  sidebar profile-name visibility, and archived-recents visibility are the only generic
  controls currently allowed to mutate the privacy-safe desktop preference file; each
  changes runtime behavior immediately. Full access and realtime voice keep their
  specialized live paths. All remaining reference controls are dimmed, explicitly
  unavailable, non-hovering, and non-clickable.
- Account and Usage render protocol data only when the connected backend supplies a
  complete snapshot. Mitsuro HTTP shows an explicit unsupported state; it does not show
  sample identities, plans, credits, limits, or billing history.
- Codex Usage renders every named bucket returned by `account/rateLimits/read`, including
  server labels, primary/secondary windows, remaining percentages, reset timestamps,
  credit balance, workspace spend control, and earned reset credits. Reset redemption
  requires a second confirmation and a fresh idempotency key; automated acceptance never
  invokes it. Workspace announcements and member credit/limit requests use the typed
  `account/workspaceMessages/read` and `account/sendAddCreditsNudgeEmail` contracts.
- Codex sign-in launches the real app-server OAuth URL and remains pending until the
  matching completion notification arrives. The user can reopen or cancel that exact
  login. Fixture device codes remain confined to explicit fixture mode.
- The composer exposes only implemented behavior: text entry, real model-gated image
  and audio-file attachments, Codex skill references and local-file mentions, Send,
  Stop, native project selection, backend-specific access presets, a searchable live
  model-catalog picker, explicit advertised reasoning-effort choices, model-advertised
  Fast mode, and backend-native Default/Build and Plan modes. The selected effort is stored per
  backend/model; Codex receives the exact
  `turn/start.effort` value and Mitsuro receives the equivalent `thinking_enabled`
  value. Codex Fast sends the advertised `serviceTier` id (`priority` in the current
  live catalog), while standard speed explicitly sends `null` to clear sticky state;
  Mitsuro sends its typed `fast_mode` boolean. Codex work modes send the exact
  `collaborationMode` preset resolved from `collaborationMode/list`; Mitsuro sends its
  typed `mode` as `build` or `plan`. Cross-backend variants are rejected before I/O.
  GPUI's native path prompt supplies
  absolute image paths; Codex receives `localImage` input and Mitsuro receives encoded
  image content. Reopened threads restore local-path, remote, and embedded image inputs
  as real transcript thumbnails; missing or unsafe image data remains visible as an
  unavailable attachment instead of disappearing. For a Codex model that advertises
  the `audio` input modality, the same picker accepts local audio files and emits the
  schema-exact `localAudio` input. Reopened local, remote, and embedded audio remains
  visible as attachment metadata. Mitsuro's HTTP content contract has no audio block,
  so the control is hidden and both product and low-level adapters reject audio before
  network I/O. Codex's add menu lists only enabled skills returned by the live backend
  and emits schema-exact `skill { name, path }` inputs; its native file picker emits
  schema-exact `mention { name, path }` inputs. Reopened threads retain both reference
  types. Mitsuro's content contract has no equivalent block, so these controls are
  hidden and both adapter layers reject them rather than converting them into prompt
  text. Codex realtime voice uses the live voice catalog plus typed start/append/stop
  calls with PipeWire capture and playback; Mitsuro hides that control because its HTTP
  contract has no realtime session API. New conversations remain local optimistic drafts until first Send so the
  selected project and access preset are present on the real session creation request;
  no synthetic server session is inserted into Recents. Codex derives the available
  built-in choices from live `permissionProfile/list`, `configRequirements/read`, and
  `modelProvider/capabilities/read` responses, then sends the schema-exact named
  permission profile on `thread/start` and `turn/start`. Full access appears only when
  the server permits it and the user has confirmed that it should be shown. Mitsuro
  maps Supervised and Autonomous to its typed `permission_mode`.
  Existing server threads show their persisted workspace read-only and require a new
  thread to change it.
- Codex server requests cannot disappear into the notification stream. Command, file,
  and exact-profile permission approvals render above the composer; structured user
  questions support options, freeform, and secret answers; standard MCP forms and URL
  elicitations receive explicit user decisions. Client-owned dynamic tools return an
  honest unsupported result when none were registered, while token refresh and
  attestation return JSON-RPC errors instead of fabricated credentials.
- Long transcripts start with a 16-message tail and reveal earlier history in bounded
  pages. Opening a Codex conversation uses `thread/resume`; moving to another task
  releases the idle prior subscription with `thread/unsubscribe`, and returning resumes
  it again. Mitsuro keeps its HTTP snapshot behavior, and an active Codex turn is never
  unsubscribed mid-stream. Find in conversation queries the selected live backend rather than filtering a
  local fixture: Codex uses `thread/searchOccurrences`, Mitsuro searches its persisted
  transcript, and an unloaded result hydrates bounded real turn pages around the exact
  returned cursor before scrolling to the item. Codex runtimes that advertise the
  methods but return JSON-RPC `-32601` use a real `thread/read(includeTurns)` projection;
  no fixture or local sample text is substituted. Reopened threads preserve structured
  reasoning, plans, commands, and file changes. The current 18-type Codex thread item
  surface is preserved across hydration and live updates: non-chat
  tool/search/image/collaboration/review lifecycle records render as real, cardless
  activity rows rather than being dropped or converted into assistant prose. Assistant
  Markdown, fenced code, visible errors, and bounded full-response expansion render
  while the composer remains pinned outside the transcript scroll.

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
environment, realtime, Remote Control, external-agent import, and background-terminal protocol families. The reviewed
`codex-cli 0.147.0` contract is checked with generated inventories: all 70
notifications must map to a typed transcript/lifecycle event, and all 11 server
requests must have an approval, interaction, or automatic transport disposition.
The GPUI shell keeps a backend-generation-scoped lifecycle subscription alive outside
active turns, refreshing account, extension, thread-list, and file state without
duplicating turn transcript events.
Active-turn steering is a transport-neutral product action: Codex uses `turn/steer`
with the exact active-turn precondition, while Mitsuro uses its durable `/chat/steer`
endpoint. A non-empty draft therefore shows Send beside Stop during a live turn.
Codex threads also expose schema-exact manual compaction from the thread overflow;
the action stays absent for Mitsuro until its HTTP API offers the same contract.
The same menu exposes a live **Review changes** action only when the Codex backend
advertises review support. It subscribes before `review/start`, streams the resulting
review turn through the native transcript and approval UI, and never substitutes
fixture review content for Mitsuro.

```bash
scripts/gpui-codex-protocol-check.sh
```

The optional `browser-native` feature links Wry/WebKitGTK. The default build uses the
external-browser bridge only while native embedding remains incomplete.
