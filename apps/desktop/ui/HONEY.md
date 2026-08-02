# Honey path for Mitsuro Desktop

Desktop product branch: `codex/desktop-product-20260726`

## What to deploy after staging -> main

1. Keep this branch isolated until main is green from staging merge.
2. Rebase or cherry-pick desktop commits onto the post-merge main/release-staging tip.
3. On Honey:
   - build/install latest Mitsuro server + hive from the release authority
   - verify `/health`
   - do not treat dirty source as runtime authority
4. Desktop UI can be previewed over private tailnet via Expo web export or Tauri package.
5. Preferred morning check:
   - local desktop UI on `:5180`
   - local/embedded server health
   - Honey runtime binary hash/unit identity for server/hive

## Local package path

```bash
cd apps/desktop/ui && bunx expo export --platform web
cd ../shell && bun run build
```

Linux packages land under `apps/desktop/shell/src-tauri/target/release/bundle/`.


## Morning checklist

Local desktop:
```bash
cd /private/tmp/mitsuro-desktop-product-20260726/apps/desktop/ui
bunx expo start --web --port 5180
# or package shell
cd ../shell && bun run dev
```

Proofs already captured:
- UI 200 on :5180
- server health 200 on :3000
- session create/load/delete smoke
- live SSE chat stream smoke (`STREAM_SMOKE_OK`)
- web export dist present

Honey after staging->main:
1. deploy latest release authority (not dirty source)
2. verify unit identity + `/health`
3. optionally host desktop web export privately or ship Tauri package


## Morning-ready evidence (local)
- branch `codex/desktop-product-20260726`
- UI :5180 200
- server :3000 health/sessions/browser/hive current 200
- chat stream DESKTOP_STREAM_OK
- web export refreshed under apps/desktop/ui/dist
- Tauri frontendDist points at desktop UI dist / :5180
