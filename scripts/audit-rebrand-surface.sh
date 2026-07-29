#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

echo "Visible legacy names and mascot language"
rg -n --hidden \
  --glob '!target/**' --glob '!node_modules/**' --glob '!.git/**' \
  --glob '!docs/rebrand/**' --glob '!scripts/audit-rebrand-surface.sh' \
  '(Krusty|KRUSTY|Mako|MAKO|krab|crab|shark)' \
  apps crates packages README.md docs 2>/dev/null || true

echo
echo "Retired visual accents"
rg -n --hidden \
  --glob '!target/**' --glob '!node_modules/**' --glob '!.git/**' \
  --glob '!docs/rebrand/**' --glob '!scripts/audit-rebrand-surface.sh' \
  '(#ff6b35|#FF6B35|orange|honey-yellow)' \
  apps crates packages README.md docs 2>/dev/null || true

echo
echo "Legacy brand asset filenames"
rg --files apps assets docs 2>/dev/null | rg -i '(krusty|mako|crab|krab|shark)' || true
