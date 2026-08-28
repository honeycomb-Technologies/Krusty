#!/usr/bin/env bash
# Map crate/app sectors by Rust/TS LOC so a tick picks one bounded path.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../../.." && pwd)"
cd "$root"

echo "SECTOR\tFILES\tLOC"

measure() {
  local path="$1"
  local pattern="$2"
  if [[ ! -d "$path" ]]; then
    return 0
  fi
  local files loc
  files="$(find "$path" -type f \( -name "$pattern" \) ! -path '*/target/*' ! -path '*/node_modules/*' ! -path '*/dist/*' | wc -l | tr -d ' ')"
  loc="$(find "$path" -type f \( -name "$pattern" \) ! -path '*/target/*' ! -path '*/node_modules/*' ! -path '*/dist/*' -print0 | xargs -0 wc -l 2>/dev/null | tail -n 1 | awk '{print $1}')"
  if [[ -z "$loc" ]]; then
    loc=0
  fi
  printf '%s\t%s\t%s\n' "$path" "$files" "$loc"
}

for crate in crates/*; do
  measure "$crate/src" '*.rs'
done

if [[ -d crates/mitsuro-core/src ]]; then
  echo
  echo "# mitsuro-core modules (prefer one of these for a tick)"
  for module in crates/mitsuro-core/src/*/; do
    measure "${module%/}" '*.rs'
  done
fi

echo
echo "# apps / packages"
measure "apps/mobile" '*.ts'
measure "apps/mobile" '*.tsx'
measure "packages" '*.ts'
measure "packages" '*.tsx'
