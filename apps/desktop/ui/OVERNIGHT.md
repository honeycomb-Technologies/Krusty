# Overnight Desktop Runbook

Branch: `codex/desktop-product-20260726`
Worktree: `/private/tmp/mitsuro-desktop-product-20260726`

## Live targets

- Desktop UI: `http://127.0.0.1:5180`
- Mitsuro server: `http://127.0.0.1:3000/health`
- Browser OS API: `http://127.0.0.1:3000/api/browser`

## Start

```bash
cd /private/tmp/mitsuro-desktop-product-20260726
./target/debug/mitsuro serve --port 3000
cd apps/desktop/ui && bunx expo start --web --port 5180
```

## Browser OS

Uses:
- browser-use (MIT) via `services/browser-use-bridge`
- BrowserOS only as AGPL UX reference (not vendored)
- Mitsuro server as shared desktop/phone control plane

Agent runs use `OPENAI_API_KEY` if set, otherwise Mitsuro-stored OpenAI credential.

```bash
# optional bridge venv
cd services/browser-use-bridge
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
```

## Smoke

```bash
curl -s http://127.0.0.1:3000/health
curl -s -H 'Authorization: Bearer local' http://127.0.0.1:3000/api/browser
# chat stream (needs allowed project dir for some workspaces)
cd apps/desktop/ui && MITSURO_SMOKE_DIR=/Users/Jacob/Documents/Mitsuro bun run smoke:stream
```

## Notes

- Ghostty is first-class terminal on desktop; embedded xterm is fallback only
- Utility panes stay closed by default
- No ornamental status strip / marketing empty states
