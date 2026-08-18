# Mitsuro Atlas browser

Atlas is a server-owned Chromium session powered by the Apache-2.0
[`agent-browser`](https://github.com/vercel-labs/agent-browser) native runtime.
The runtime is pinned to `0.34.0` and packaged beside the Mitsuro server.

## Control plane

- Honey owns browser processes and session lifecycle.
- Raw CDP and the native agent-browser stream bind only to loopback.
- `/api/browser/:id/stream` is the authenticated remote WebSocket proxy.
- The stream uses latest-frame delivery, acknowledgement pacing, and a client
  frame-rate cap so a slow phone does not accumulate stale frames.
- One controller lease can send mouse, keyboard, or touch events. Any number of
  viewer leases can receive frames without injecting input.
- Agent-facing HTTP automation is an allowlisted semantic action batch. It does
  not expose shell arguments, arbitrary JavaScript evaluation, raw CDP, Chrome
  launch flags, profile paths, or downloads.

## API

- `GET /api/browser`
- `POST /api/browser` `{ title?, kind?: "interactive" | "agent", url? }`
- `GET /api/browser/:id`
- `POST /api/browser/:id/stop`
- `POST /api/browser/:id/heartbeat` `{ capability: "viewer" | "controller", client_id? }`
- `POST /api/browser/:id/actions` `{ actions: [...] }`
- `POST /api/browser/:id/agent` `{ task, model?, max_steps? }`
- `GET /api/browser/:id/stream?capability=viewer|controller` (WebSocket)

Supported semantic actions include navigate, snapshot, click, fill, type,
press, hover, select, scroll, history, reload, bounded waits, and page queries.

## Runtime installation

For a source checkout:

```bash
sh scripts/install-atlas-runtime.sh
```

The installer downloads the exact pinned npm archive, verifies its SHA-256,
and stages only the native binary at `target/atlas/agent-browser`. Release
archives include the same binary beside `mitsuro`, so no Node or Python runtime
is required after packaging. Atlas discovers an existing Chrome/Chromium first;
on a host without one, run:

```bash
target/atlas/agent-browser install
```

`MITSURO_AGENT_BROWSER_PATH` can point Honey at an operator-managed binary.
The shipped user unit sets that to
`~/.local/bin/.mitsuro-current/agent-browser`. If a live Honey host returns
`API 503` for Browser, run `sh scripts/honey-atlas-repair.sh` instead of
rebuilding from source.

## Natural-language agent runs

The semantic action API is always available when the sidecar is installed.
The optional natural-language `chat` runner additionally requires
`AI_GATEWAY_API_KEY`; its eval and download action categories are denied in
the non-interactive server process. Mitsuro returns a structured error when the
credential is absent instead of silently switching providers.

## Clients

- Desktop/web renders the authenticated Atlas stream and forwards direct
  pointer, wheel, and keyboard input.
- Mobile renders the same authoritative remote Chromium session and forwards
  touch plus keyboard input.
- Local development previews remain a separate lightweight WebView/iframe path;
  they are not presented as authoritative browser-agent sessions.
