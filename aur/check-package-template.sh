#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
pkgbuild="$script_dir/PKGBUILD"
srcinfo="$script_dir/.SRCINFO"
updater="$script_dir/update-release.sh"
repo_root=$(cd -- "$script_dir/.." && pwd)
rendered_dir=$(mktemp -d "${TMPDIR:-/tmp}/mitsuro-aur-units.XXXXXX")
trap 'rm -rf -- "$rendered_dir"' EXIT

# PKGBUILD is declarative until makepkg calls one of its functions, so loading
# it here is a deterministic way to compare its metadata with .SRCINFO.
# shellcheck disable=SC1090
source "$pkgbuild"

[[ "$pkgname" == mitsuro ]]
[[ "$url" == https://github.com/honeycomb-Technologies/Mitsuro ]]
[[ "$pkgdesc" == *Mitsuro*Hive* ]]
[[ " ${provides[*]} " == *" krusty "* ]]
[[ " ${conflicts[*]} " == *" krusty "* ]]

if [[ "$pkgver" == UNRELEASED ]]; then
  [[ ${#source[@]} -eq 0 ]]
  [[ ${#sha256sums[@]} -eq 0 ]]
  grep -Fq 'UNRELEASED TEMPLATE' "$pkgbuild"
  error() { :; }
  if prepare; then
    printf 'unreleased PKGBUILD unexpectedly passed prepare()\n' >&2
    exit 1
  fi
else
  [[ "$pkgver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
  [[ ${#source[@]} -eq 1 ]]
  [[ "${source[0]}" == \
    "mitsuro-$pkgver.tar.gz::https://github.com/honeycomb-Technologies/Mitsuro/archive/refs/tags/v$pkgver.tar.gz" ]]
  [[ ${#sha256sums[@]} -eq 1 ]]
  [[ "${sha256sums[0]}" =~ ^[0-9a-f]{64}$ ]]
fi

grep -Fq 'cargo build --frozen --release -p mitsuro' "$pkgbuild"
grep -Fq 'cargo build --frozen --release -p mitsuro-hive' "$pkgbuild"
grep -Fq 'deploy/systemd/mitsuro-hive.service' "$pkgbuild"
grep -Fq 'deploy/systemd/mitsuro-serve.service' "$pkgbuild"
grep -Fq "sed 's|^ExecStart=%h/.local/bin/.mitsuro-current/|ExecStart=/usr/bin/|'" \
  "$pkgbuild"
if grep -Eq 'crates/(krusty|krusty-mako)' "$pkgbuild"; then
  printf 'PKGBUILD refers to a previous-generation source path\n' >&2
  exit 1
fi

for portable_service in mitsuro-hive.service mitsuro-serve.service; do
  portable_path="$repo_root/deploy/systemd/$portable_service"
  ! grep -Eq '^ExecStart=/usr/bin/env([[:space:]]|$)' "$portable_path"
  grep -Eq '^ExecStart=%h/\.local/bin/\.mitsuro-current/mitsuro(-hive)?([[:space:]]|$)' \
    "$portable_path"
  sed 's|^ExecStart=%h/.local/bin/.mitsuro-current/|ExecStart=/usr/bin/|' \
    "$portable_path" > "$rendered_dir/$portable_service"
  ! grep -Eq '^ExecStart=(/usr/bin/env|%h/)' "$rendered_dir/$portable_service"
done
grep -Eq '^ExecStart=/usr/bin/mitsuro-hive([[:space:]]|$)' \
  "$rendered_dir/mitsuro-hive.service"
grep -Eq '^ExecStart=/usr/bin/mitsuro([[:space:]]|$)' \
  "$rendered_dir/mitsuro-serve.service"

if command -v makepkg >/dev/null 2>&1; then
  diff -u "$srcinfo" <(cd -- "$script_dir" && makepkg --printsrcinfo)
else
  grep -Fqx $'pkgbase = mitsuro' "$srcinfo"
  grep -Fqx $'\tpkgver = '"$pkgver" "$srcinfo"
  grep -Fqx $'\tpkgrel = '"$pkgrel" "$srcinfo"
  grep -Fqx $'\tprovides = krusty' "$srcinfo"
  grep -Fqx $'\tconflicts = krusty' "$srcinfo"
  if [[ "$pkgver" == UNRELEASED ]]; then
    ! grep -q $'^\tsource = ' "$srcinfo"
    ! grep -q $'^\tsha256sums = ' "$srcinfo"
  else
    grep -Fqx $'\tsource = '"${source[0]}" "$srcinfo"
    grep -Fqx $'\tsha256sums = '"${sha256sums[0]}" "$srcinfo"
  fi
fi

bash -n "$updater"
if "$updater" UNRELEASED >/dev/null 2>&1; then
  printf 'AUR updater accepted a non-release version\n' >&2
  exit 1
fi

printf 'AUR package template check passed (%s)\n' "$pkgver"
