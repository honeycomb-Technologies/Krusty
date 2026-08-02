# Mitsuro Browser OS (Atlas)

## What we use
- **browser-use** (MIT): real agent automation against session CDP endpoints via `services/browser-use-bridge`.
- **BrowserOS** (AGPL-3.0): UX/protocol reference only. Not vendored, not forked, not linked into Mitsuro.
- **krusty-server**: session registry + remote plane so desktop and phone share one browser control plane.

## API
- `GET /api/browser`
- `POST /api/browser` `{ title?, kind?: interactive|agent, url?, launch_local? }`
- `GET /api/browser/:id`
- `POST /api/browser/:id/stop`
- `POST /api/browser/:id/heartbeat` `{ capability?: viewer|controller }`
- `POST /api/browser/:id/agent` `{ task, model?, max_steps? }` → browser-use bridge

## Install bridge (optional for agent runs)
```bash
cd services/browser-use-bridge
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
playwright install chromium
```

## Notes
- Local Chromium is launched by the server with `--remote-debugging-port`.
- Desktop cockpit is in `apps/desktop/ui/src/browser/DesktopBrowserPane.tsx`.
- Phone remotes attach through the same Mitsuro server; no desktop-only browser state.

## Agent credentials
browser-use needs an LLM key in the server environment, typically:
```bash
export OPENAI_API_KEY=...
# optional
export KRUSTY_BROWSER_USE_MODEL=gpt-4.1-mini
export KRUSTY_BROWSER_USE_PYTHON=/path/to/services/browser-use-bridge/.venv/bin/python
```
Without a key, session create + CDP attach still work; agent runs return a structured credential error.
