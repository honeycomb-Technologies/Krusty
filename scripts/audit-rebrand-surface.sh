#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

failures=0

fail() {
  printf 'FAIL  %s\n' "$1"
  failures=$((failures + 1))
}

pass() {
  printf 'PASS  %s\n' "$1"
}

require_file() {
  if [[ -f "$1" ]]; then
    pass "$1 exists"
  else
    fail "$1 is missing"
  fi
}

require_text() {
  local label="$1"
  local pattern="$2"
  local path="$3"
  if rg -q "$pattern" "$path"; then
    pass "$label"
  else
    fail "$label"
  fi
}

reject_matches() {
  local label="$1"
  local pattern="$2"
  shift 2
  local output
  if output="$(rg -n --hidden \
    --glob '!target/**' \
    --glob '!node_modules/**' \
    --glob '!dist/**' \
    --glob '!build/**' \
    --glob '!.git/**' \
    --glob '!docs/archive/**' \
    --glob '!**/*.lock' \
    --glob '!**/test/**' \
    --glob '!**/tests/**' \
    --glob '!**/*.test.*' \
    "$pattern" "$@" 2>/dev/null)"; then
    fail "$label"
    printf '%s\n' "$output"
  else
    pass "$label"
  fi
}

printf 'Mitsuro v1 presentation audit\n'

if git diff --quiet -- crates/krusty-cli/src/tui; then
  pass "TUI source is untouched in the current worktree"
else
  fail "TUI source changed even though its rebuild is intentionally deferred"
  git diff --stat -- crates/krusty-cli/src/tui
fi

for asset in \
  assets/branding/mitsuro/mitsuro-cell-flat.svg \
  assets/branding/mitsuro/mitsuro-cell-mono.svg \
  assets/branding/mitsuro/mitsuro-cell-dimensional.svg \
  assets/branding/mitsuro/mitsuro-wordmark.svg \
  assets/branding/mitsuro/mitsuro-lockup-horizontal.svg \
  assets/branding/mitsuro/mitsuro-hive.svg \
  assets/branding/mitsuro/mitsuro-app-icon-master.svg \
  assets/branding/mitsuro/mitsuro-app-icon-tinted.svg \
  assets/branding/mitsuro/mitsuro-adaptive-foreground.svg \
  apps/mobile/assets/animations/splash.json \
  apps/mobile/assets/icons/mitsuro-notification.png \
  apps/desktop/shell/src-tauri/icons/icon.icns \
  apps/desktop/shell/src-tauri/icons/icon.ico
do
  require_file "$asset"
done

for retired in \
  assets/branding/krusty-k.png \
  icons/krusty-k.svg \
  icons/krusty-k-theme.svg \
  icons/mako-shark.svg \
  apps/mobile/app/navigation-preview.tsx
do
  if [[ -e "$retired" ]]; then
    fail "$retired must remain retired"
  else
    pass "$retired is retired"
  fi
done

require_text "Expo display name is Mitsuro" '"name"[[:space:]]*:[[:space:]]*"Mitsuro"' apps/mobile/app.json
require_text "Expo notification icon is Mitsuro" 'mitsuro-notification\.png' apps/mobile/app.json
require_text "desktop product name is Mitsuro" '"productName"[[:space:]]*:[[:space:]]*"Mitsuro"' apps/desktop/shell/src-tauri/tauri.conf.json
require_text "desktop publisher is Honeycomb Technologies" '"publisher"[[:space:]]*:[[:space:]]*"Honeycomb Technologies"' apps/desktop/shell/src-tauri/tauri.conf.json
require_text "preferred autonomous CLI is Hive" 'name = "hive"' crates/krusty-cli/src/main.rs
require_text "legacy autonomous CLI alias is retained" 'visible_alias = "mako"' crates/krusty-cli/src/main.rs
require_text "shared accent is mineral violet" '#75617e' packages/ui/src/tokens.ts
require_text "shared foundation is graphite" '#0e0e11' packages/ui/src/tokens.ts
require_text "shared foreground is wax" '#e8e5ea' packages/ui/src/tokens.ts
require_text "Hive service metadata is branded" 'Description=Mitsuro Hive' deploy/systemd/krusty-mako.service

reject_matches \
  "production frontends contain no retired mascot entry points" \
  '(CrabIcon|MakoSharkIcon|KrustyLogo|krusty-k|mako-shark)' \
  apps/mobile apps/desktop apps/website packages

reject_matches \
  "production frontends contain no retired exact display labels" \
  '("(Krusty|Mako)"|'\''(Krusty|Mako)'\''|>(Krusty|Mako)<)' \
  apps/mobile apps/desktop apps/website packages

reject_matches \
  "production frontends contain no retired orange, navy, or OAuth-gradient colors" \
  '(#ff6b35|#FF6B35|#e17a30|#E17A30|#1a1f2e|#151b2b|#101827|#667eea|#764ba2)' \
  apps/mobile apps/desktop apps/website packages \
  crates/krusty-core/src/auth/browser_flow/callback_server.rs \
  crates/krusty-server/src/routes/oauth/callback.rs

reject_matches \
  "active public copy contains no retired mascot slogans" \
  '(Always Swimming|Set course|Schedule course|Krusty the Krab|Horseshoe Crab|Mantis Shrimp)' \
  apps/mobile apps/desktop apps/website packages README.md docs \
  crates/krusty-cli/src/main.rs crates/krusty-cli/src/serve.rs

printf '\nCompatibility identifiers intentionally retained\n'
printf '  binary/package: krusty, krusty-mako, @krusty/*\n'
printf '  transport/state: /mako, session_type=mako, ~/.krusty, krusty://\n'
printf '  platform IDs: existing bundle IDs, Expo slug, native module symbols\n'
printf '  source symbols: Mako* and Krusty* types until a separate migration\n'
printf '  archive docs: historical names are preserved as historical evidence\n'
printf '  TUI: legacy visual surface is frozen pending a ground-up rebuild\n'

if (( failures > 0 )); then
  printf '\nMitsuro presentation audit failed with %d issue(s).\n' "$failures"
  exit 1
fi

printf '\nMitsuro presentation audit passed.\n'
