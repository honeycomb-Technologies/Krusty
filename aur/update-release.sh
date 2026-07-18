#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 <released-version-without-v>" >&2
    echo "example: $0 0.8.0" >&2
}

if [[ $# -ne 1 ]]; then
    usage
    exit 2
fi

version=${1#v}
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "invalid release version: $1" >&2
    exit 2
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
pkgbuild="$script_dir/PKGBUILD"
srcinfo="$script_dir/.SRCINFO"
archive_name="krusty-$version.tar.gz"
archive_url="https://github.com/honeycomb-Technologies/Krusty/archive/refs/tags/v$version.tar.gz"
expected_prefix="Krusty-$version/"
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/krusty-aur-release.XXXXXX")
trap 'rm -rf -- "$temp_dir"' EXIT

curl \
    --proto '=https' \
    --tlsv1.2 \
    --fail \
    --location \
    --silent \
    --show-error \
    --output "$temp_dir/$archive_name" \
    "$archive_url"

first_entry=$(tar -tzf "$temp_dir/$archive_name" | sed -n '1p')
if [[ "$first_entry" != "$expected_prefix" ]]; then
    echo "unexpected archive root: $first_entry (expected $expected_prefix)" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    checksum=$(sha256sum "$temp_dir/$archive_name" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    checksum=$(shasum -a 256 "$temp_dir/$archive_name" | awk '{print $1}')
else
    echo "sha256sum or shasum is required" >&2
    exit 1
fi

update_file() {
    local source_file=$1
    local temp_file
    temp_file=$(mktemp "$script_dir/.aur-update.XXXXXX")
    awk \
        -v version="$version" \
        -v checksum="$checksum" \
        -v archive_name="$archive_name" \
        -v archive_url="$archive_url" \
        -v file_kind="$2" '
        file_kind == "pkgbuild" && /^pkgver=/ {
            print "pkgver=" version
            next
        }
        file_kind == "pkgbuild" && /^sha256sums=/ {
            print "sha256sums=(\047" checksum "\047)"
            next
        }
        file_kind == "srcinfo" && /^\tpkgver = / {
            print "\tpkgver = " version
            next
        }
        file_kind == "srcinfo" && /^\tsource = / {
            print "\tsource = " archive_name "::" archive_url
            next
        }
        file_kind == "srcinfo" && /^\tsha256sums = / {
            print "\tsha256sums = " checksum
            next
        }
        { print }
    ' "$source_file" > "$temp_file"
    chmod 0644 "$temp_file"
    mv -- "$temp_file" "$source_file"
}

update_file "$pkgbuild" pkgbuild
update_file "$srcinfo" srcinfo

grep -Fqx "pkgver=$version" "$pkgbuild"
grep -Fqx "sha256sums=('$checksum')" "$pkgbuild"
grep -Fqx $'\tpkgver = '"$version" "$srcinfo"
grep -Fqx $'\tsha256sums = '"$checksum" "$srcinfo"

if command -v makepkg >/dev/null 2>&1; then
    generated_srcinfo=$(mktemp "$script_dir/.srcinfo-check.XXXXXX")
    trap 'rm -rf -- "$temp_dir" "$generated_srcinfo"' EXIT
    (
        cd -- "$script_dir"
        makepkg --printsrcinfo
    ) > "$generated_srcinfo"
    if ! cmp -s "$generated_srcinfo" "$srcinfo"; then
        echo "generated .SRCINFO differs; replacing the checked-in metadata" >&2
        chmod 0644 "$generated_srcinfo"
        mv -- "$generated_srcinfo" "$srcinfo"
    fi
fi

echo "Updated AUR metadata for v$version"
echo "SHA-256: $checksum"
echo "Verify with: makepkg --verifysource -f"
