# GPUI desktop status and continuation

The GPUI desktop client is maintained in `apps/desktop/gpui` on the same Rust
workspace as the Mitsuro server and client contracts. The transport boundary is
`crates/mitsuro-desktop-backend`.

The former standalone prototype directory is a preserved local snapshot. Do not
resume implementation there. Its build target,
gauntlet reports, reverse-engineering dumps, and duplicate comparison captures
are not canonical product evidence.

## Backend selection

`MITSURO_BACKEND` accepts:

- `mitsuro-http` (default): `MITSURO_SERVER_URL`, default `http://127.0.0.1:3000`;
  optional `MITSURO_SERVER_TOKEN` for authenticated remote access.
- `codex-stdio`: managed `codex app-server --stdio`; optional `CODEX_BIN`.
- `fixture`: explicit deterministic UI development.
- `auto`: currently prefers Mitsuro HTTP. Probe/fallback policy still needs a
  product decision.
- `codex-ws`: reserved and rejected honestly until the WebSocket transport is
  implemented.

A Ready, authenticated product backend sends a provider-backed turn by default.
Use `MITSURO_NO_LIVE_TURN=1` for read-only UI review or an explicit fixture backend for
deterministic replay. Merely connecting and discovering auth does not itself send a turn.
`MITSURO_NO_LIVE_TURN` disables Send; it does not select fixture replay.

## Production data invariant

Fixture records are allowed only when both the UI connection and active backend are
explicitly `fixture`. Mitsuro HTTP, Codex stdio/WebSocket, connecting, and error states
cannot use fixture catalogs or fixture process/file/session operations. Backend errors
clear the affected projection and render a typed loading, empty, unsupported, or error
state. They never substitute sample success.

Desktop preferences, current mode, local browser URL history, and transient composer UI
remain local application state. Server-owned sessions, transcripts, models, files,
processes, extensions, Hive runs/schedules, and account usage come from the selected
backend or are shown as unavailable.

## Truth matrix

| Surface | Mitsuro HTTP | Codex stdio | Fixture/static status |
|---|---|---|---|
| Health/connect | Live | Live | Explicit fixture |
| Sessions list/create/read/rename/delete | Live | Live | Typed fixture |
| Streaming chat | Live SSE + durable steering | Live JSON-RPC notifications + `turn/steer` | Sample replay |
| Image attachments | Live base64 image content | Live schema-exact `localImage` input | Unsupported, hidden |
| Audio-file attachments | Unsupported, hidden and rejected before I/O | Live schema-exact `localAudio`, selected-model gated | Unsupported, hidden |
| Skill references | Unsupported, hidden and rejected before I/O | Live schema-exact `skill { name, path }` from enabled server skills | Unsupported, hidden |
| File mentions | Unsupported, hidden and rejected before I/O | Live schema-exact `mention { name, path }` from native file picker | Unsupported, hidden |
| Models | Live | Live | Typed fixture catalog |
| Reasoning effort | Live model-advertised `thinking_enabled` | Live model-advertised `turn/start.effort` | Typed fixture options |
| Project/workspace | Native folder picker; selected path becomes `working_dir` on real session creation | Native folder picker; absolute `cwd` and runtime workspace root on real session creation | Hidden |
| Access mode | Live typed `permission_mode`: Supervised or Autonomous | Live schema-exact Read-only, Auto, or Full access approval/sandbox preset | Hidden |
| Interrupt | Live session cancel | Live turn interrupt | Typed fixture |
| Tool approval | Live | Live command, file, and exact-profile permission decisions | Sample approval |
| Archive/unarchive | Unsupported, capability-gated | Live | Typed fixture |
| Fork | Unsupported, capability-gated | Live | Typed fixture |
| Review changes | Unsupported, capability-gated | Live streamed `review/start` with approvals | Unsupported, hidden |
| Account authentication | Unsupported, unavailable state | Live browser OAuth, cancel, completion notification, logout | Explicit offline fixture only |
| Files | Live tree/read/fuzzy adapter | Live typed paths | Typed fixture |
| Processes | Read-only server catalog in client; interactive terminal spawn unsupported | Live spawn/stdin/PTY | Typed fixture |
| Extensions/MCP/skills/hooks | Live read-only installed extensions, MCP status, and skills; plugin mutations, configuration writes, OAuth, and hooks unsupported | Live catalog, typed plugin install/uninstall, MCP OAuth login, HTTP/stdio MCP configuration writes, MCP status, skills, and per-workspace hooks | Typed read-only fixture; no sample hooks |
| Hive/schedules | Live read-only projections; mutations disabled | Unsupported | Typed fixture UI |
| Pull requests | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Sites | No product adapter; explicit unavailable state | No product adapter; explicit unavailable state | No fake catalog |
| Browser | System-browser bridge; no page ownership | System-browser bridge; no page ownership | Same local bridge |
| Computer environments/permissions | Unsupported; no invented rows or grants | Live environment add/status/info and exact requested permission grants; no list method | Explicit fixture catalog, labeled fixture |
| Settings writes | Desktop preferences persist locally; server config writes unsupported | Desktop preferences persist locally; MCP add persists through typed config write/reload; other server settings unsupported | Same local persistence boundary |

Unsupported operations must return `NotImplemented` or be disabled through
`BackendCapabilities`. A method name appearing in the Codex inventory does not make it
implemented. The committed `codex-cli 0.147.0` baseline contains 95 stable client
methods, 133 methods with experimental APIs enabled, 70 server notifications, 10 server
requests in the stable contract (11 with experimental APIs), and 18 thread item
variants. The desktop automatically answers the experimental `currentTime/read` server
request. Every generated notification is classified as a core transcript event or a
typed lifecycle event. Every generated server request has an explicit disposition:
native approval/user-input/MCP interaction, a structured unsupported dynamic-tool
result, or an honest JSON-RPC error for unadvertised token/attestation capabilities.
The desktop negotiates experimental APIs because its process, environment, realtime,
and background-terminal surfaces require them. Fixture `call_raw` no longer manufactures
generic success payloads.

The executable client-method coverage matrix currently identifies 63 typed adapters
and 70 raw-transport-only methods. Raw reachability is treated as remaining product
work, not as feature completion; the matrix test must change with each typed adapter.

## Established recovery baseline

- GPUI and GPUI Component resolve from crates.io; there is no machine-specific
  `/home/.../vendor/gpui` patch.
- The Extensions marketplace renders only backend data. Codex plugin Install/Remove
  actions call typed `plugin/install` and `plugin/uninstall`, disable concurrent
  mutations, and refresh the live catalog after success. Mitsuro and explicit fixture
  catalogs are visibly read-only because neither backend exposes a production mutation
  contract. Search filters the live plugin, skill, and MCP records; expanding a category
  uses its exact hidden-record count and never pads the catalog with decorative totals.
- Codex remote-environment registration sends exact `environment/add` parameters after
  local `ws://`/`wss://` validation. Because the protocol returns an empty response and
  has no list method, GPUI retains only successful submissions for the current app
  session and immediately probes `environment/status` and `environment/info`. Mitsuro
  renders the mutation as unsupported.
- MCP servers advertising `notLoggedIn` expose a real Codex sign-in action. GPUI sends
  typed `mcpServer/oauth/login`, opens only the returned authorization URL, tracks the
  server name until `mcpServer/oauthLogin/completed`, and refreshes the live catalog.
- Codex MCP additions use typed `config/value/write` with the exact
  `mcp_servers.<name>` upsert shape, followed by `config/mcpServer/reload`. The GPUI form
  supports streamable HTTP URLs and stdio command plus JSON string-array arguments,
  validates all fields before I/O, serializes one mutation at a time, and refreshes the
  live catalog after success. Mitsuro renders this mutation as unsupported.
- Settings Hooks renders only typed `hooks/list` entries scoped to the active workspace.
  It preserves hook event, handler, source path/source, enabled/managed state, trust,
  warnings, and errors. The previous local toggle and hard-coded Mitsuro paths were
  removed because they were not connected to either backend.
- Settings Connections hydrates the real Codex app/connector catalog through typed
  `app/list` and `app/installed` adapters, with `app/read` available for exact detail
  reads. Rows derive connected, installed, disabled, accessible, and unavailable states
  only from server fields. Connect actions validate and open the exact server-returned
  HTTP(S) install URL. Mitsuro renders this Codex-only capability as unsupported, and
  explicit fixture mode does not invent connector account state.
- Codex notifications use an application-lifetime broadcast hub. Independent
  turn subscribers do not consume each other's events. The GPUI shell owns one
  backend-generation-scoped lifecycle subscriber for idle-time account, skills/MCP,
  thread-list, and file updates; turn transcript deltas remain single-delivery.
- Reopened and live Codex transcripts preserve every current thread item type. Tool,
  search, image, collaboration, review-mode, compaction, hook, and sleep items render as
  real activity rows; unknown future variants remain visible rather than disappearing.
- Server-originated requests are modeled separately from notifications because Codex
  can pause a turn until the client answers. The native interaction strip supports
  request-user-input options/freeform/secrets, standard MCP forms and URL elicitations,
  plus command/file/permission approvals without synthetic response data.
- Codex review turns subscribe before `review/start`, follow the response's review
  thread identity, and reuse the progressive transcript and approval pipeline. The
  selected-thread action requests an inline review of real uncommitted changes; it is
  hidden for Mitsuro until the HTTP API exposes an equivalent contract.
- Codex account sign-in starts the real browser OAuth flow, retains the server login
  identity for cancel/reopen, and waits for `account/login/completed` before claiming
  success or loading authenticated usage. Failed logout keeps the last server snapshot
  visible instead of pretending local sign-out succeeded.
- The composer path picker accepts up to four supported images at 20 MiB each. Codex
  receives absolute `localImage` user inputs; Mitsuro reads the selected file and sends
  a real MIME-labeled base64 content block. Both thread-read adapters preserve persisted
  local-path, remote, and embedded image inputs, and GPUI renders them in the originating
  user message. Missing files, failed remote loads, invalid MIME types, oversized data,
  and decode failures use explicit visual fallbacks. Attachments are cleared on backend
  switch and never fall back to fixture data.
- Codex models advertising the `audio` input modality expose a local audio-file picker
  using the same limit of four combined attachments at 20 MiB per file. The product
  adapter emits `localAudio`; persisted local-path, remote, and embedded audio inputs are
  retained as truthful transcript attachment rows. Invalid or oversized embedded data
  stays visible as unavailable metadata. Mitsuro's text/image `ContentBlock` contract
  does not accept audio, so its UI action is hidden and both adapter layers reject it
  before I/O. This is file attachment support, not microphone recording or playback.
- Codex's composer add menu uses the live enabled-skill catalog and a native regular-file
  picker to emit exact `skill { name, path }` and `mention { name, path }` inputs. Up to
  eight combined references are retained as structured user-message attachments when a
  thread is reopened. Mitsuro and fixture modes hide these actions and both product and
  low-level Mitsuro adapters reject the Codex-only records; they are never flattened into
  prompt text or synthesized from fixture data.
- Reasoning choices come only from the selected model's live capability metadata and
  persist per backend/model. The transport-neutral turn adapter maps the same selection
  to Codex `effort` and Mitsuro `thinking_enabled`; models with zero or one advertised
  option do not show a misleading selector.
- Project and access controls are real product adapters rather than prompt decoration.
  A new conversation is an optimistic local draft until first Send, when the selected
  absolute project path and backend-specific access preset are used for the real session
  and first turn. Codex Read-only/Auto/Full access maps to exact approval, reviewer,
  sandbox, and runtime-root fields. Mitsuro Supervised/Autonomous maps to its typed
  permission contract. Transport-only Mitsuro metadata is skipped from Codex JSON, and
  cross-backend access variants are rejected before I/O.
- Mitsuro uses the canonical `mitsuro-client` HTTP/SSE implementation.
- Thread reads preserve the canonical transcript rather than limiting history to
  eight 280-character bubbles.
- Backend session IDs are namespaced (`BackendSessionId`) and are stored on every
  live GPUI thread. Session/model/turn flows use the transport-neutral
  `ProductBackend` contract, and mutations reject a session whose origin differs
  from the active backend.
- The current UI still shows one selected backend at a time. A future mixed-backend
  list must use `BackendSessionId::qualified()` as its row/selection key instead of
  the raw server ID.

## Release posture

- The fixture/live visual matrix, dual-provider acceptance gauntlet, full workspace
  gates, optimized GPUI build, and isolated runtime provenance check are complete.
- The production-data purity matrix was revalidated on both live transports for Work,
  Scheduled, Computer, Extensions, Settings, and Files. A unit regression matrix forbids
  fixture records for every product backend connection state.
- Codex WebSocket stays explicitly unsupported unless attaching to an already-running
  app-server becomes a required deployment mode; managed stdio is the supported path.
- Installation and deployment are separate operator actions. A built GPUI artifact is
  not evidence that the installed Mitsuro CLI/server or any live process changed.
- Splitting `app.rs` into state/controllers and bounded views is a post-release
  maintainability slice and must not change the backend capability boundary.

## Validation

```bash
cargo check -p mitsuro-desktop-backend
cargo test -p mitsuro-client -p mitsuro-desktop-backend
cargo check -p mitsuro-gpui-desktop
cargo test -p mitsuro-gpui-desktop
scripts/gpui-codex-protocol-check.sh

# Read-only check against a running local Mitsuro server
MITSURO_RUN_SERVER_IT=1 cargo test -p mitsuro-desktop-backend \
  live_server_read_only_contract -- --nocapture
```
