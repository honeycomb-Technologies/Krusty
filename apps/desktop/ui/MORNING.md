# Mitsuro Desktop — Morning Ready

Branch: `codex/desktop-product-20260726`
Worktree: `/private/tmp/mitsuro-desktop-product-20260726`

## What is ready now
- Dedicated desktop product branch from latest dirty Mitsuro
- Desktop-first shell: Chat / Code / Hive plane rail + context rail + canvas + utility host
- Production reuse: ChatTranscript, ChatBar, SettingsPanel, HiveScreen, shared `@mitsuro/api|state`
- Browser OS: server-backed `/api/browser` + browser-use bridge (MIT), BrowserOS AGPL reference only
- Ghostty-first terminal surface
- Auto-auth local server (`http://127.0.0.1:3000` + token `local`)
- Web export present at `apps/desktop/ui/dist`
- Tauri shell retargeted to desktop UI port `5180`

## Local start
```bash
cd /private/tmp/mitsuro-desktop-product-20260726
./target/debug/mitsuro-hive >/tmp/mitsuro-desktop-runtime/hive.log 2>&1 &
./target/debug/mitsuro serve --port 3000 >/tmp/mitsuro-desktop-runtime/server.log 2>&1 &
cd apps/desktop/ui && bunx expo start --web --port 5180
```
Open http://127.0.0.1:5180

## Packaging path
```bash
cd apps/desktop/ui && bun run web:build
cd ../shell && bun run dev   # Tauri shell
# Linux package:
# cd ../shell && bun run build
```

## Verified live
- UI 200 on :5180
- health 200 on :3000
- sessions + chat stream (`DESKTOP_STREAM_OK`)
- browser session create/list
- hive current endpoint
- hive daemon socket healthy

## Honey note
Do **not** deploy this dirty branch as production authority.
After staging → main:
1. rebase/cherry-pick desktop commits onto release authority
2. deploy server/hive from that authority on Honey
3. verify unit identity + `/health`
4. ship desktop web export privately or Tauri package against that server
