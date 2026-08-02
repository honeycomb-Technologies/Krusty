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
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "invalid stable release version: $1 (AUR pkgver values cannot contain '-')" >&2
    exit 2
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
pkgbuild="$script_dir/PKGBUILD"
srcinfo="$script_dir/.SRCINFO"
archive_name="mitsuro-$version.tar.gz"
archive_url="https://github.com/honeycomb-Technologies/Mitsuro/archive/refs/tags/v$version.tar.gz"
expected_prefix="Mitsuro-$version/"
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/mitsuro-aur-release.XXXXXX")
trap 'rm -rf -- "$temp_dir"' EXIT
archive_listing="$temp_dir/archive-entries.txt"
archive_type_listing="$temp_dir/archive-entry-types.txt"
archive_regular_listing="$temp_dir/archive-regular-entries.txt"
source_manifest="$temp_dir/mitsuro-cli-Cargo.toml"

tar_bin=${TAR:-tar}
if ! command -v "$tar_bin" >/dev/null 2>&1; then
    echo "GNU tar is required to validate the release archive" >&2
    exit 1
fi
tar_version=$("$tar_bin" --version 2>/dev/null || true)
if [[ "${tar_version%%$'\n'*}" != *"GNU tar"* ]]; then
    echo "GNU tar is required to validate the release archive (set TAR to its path)" >&2
    exit 1
fi
if ! command -v makepkg >/dev/null 2>&1; then
    echo "makepkg is required to generate synchronized AUR metadata" >&2
    exit 1
fi

curl \
    --proto '=https' \
    --tlsv1.2 \
    --fail \
    --location \
    --silent \
    --show-error \
    --output "$temp_dir/$archive_name" \
    "$archive_url"

LC_ALL=C "$tar_bin" --absolute-names --quoting-style=escape -tzf \
    "$temp_dir/$archive_name" > "$archive_listing"
LC_ALL=C "$tar_bin" --absolute-names --quoting-style=escape -tvzf \
    "$temp_dir/$archive_name" > "$archive_type_listing"
first_entry=$(sed -n '1p' "$archive_listing")
if [[ "$first_entry" != "$expected_prefix" ]]; then
    echo "unexpected archive root: $first_entry (expected $expected_prefix)" >&2
    exit 1
fi

manifest_entry="${expected_prefix}crates/mitsuro-cli/Cargo.toml"
if ! LC_ALL=C awk -v prefix="$expected_prefix" -v manifest="$manifest_entry" '
    function unsafe_path(path, normalized, count, components, index_) {
        if (path == "" || substr(path, 1, 1) == "/" || path ~ /\\/) {
            return 1
        }

        normalized = path
        if (substr(normalized, length(normalized), 1) == "/") {
            normalized = substr(normalized, 1, length(normalized) - 1)
        }
        count = split(normalized, components, "/")
        for (index_ = 1; index_ <= count; index_++) {
            if (components[index_] == "" || components[index_] == "." ||
                    components[index_] == ".." ||
                    components[index_] ~ /^[A-Za-z]:/) {
                return 1
            }
        }
        return 0
    }

    {
        entry = $0
        if (unsafe_path(entry) || seen[entry]++) {
            invalid = 1
            exit 1
        }
        if (entry == prefix) {
            root_count++
        } else if (index(entry, prefix) != 1) {
            invalid = 1
            exit 1
        }
        if (entry == manifest) {
            manifest_count++
        }
    }
    END {
        if (invalid || NR == 0 || root_count != 1 || manifest_count != 1) {
            exit 1
        }
    }
' "$archive_listing"; then
    echo "release archive contains an unsafe, ambiguous, duplicate, or out-of-root path" >&2
    exit 1
fi

if ! LC_ALL=C awk -v prefix="$expected_prefix" -v manifest="$manifest_entry" '
    FNR == NR {
        paths[++path_count] = $0
        next
    }
    {
        type_count++
        type = substr($0, 1, 1)
        path = paths[type_count]
        if (type != "-" && type != "d") {
            invalid = 1
        }
        if (path == prefix && type != "d") {
            invalid = 1
        }
        if (path == manifest && type != "-") {
            invalid = 1
        }
        if (type == "-" && substr(path, length(path), 1) == "/") {
            invalid = 1
        }
        if (type == "-") {
            print path
        }
    }
    END {
        if (invalid || path_count == 0 || path_count != type_count) {
            exit 1
        }
    }
' "$archive_listing" "$archive_type_listing"; then
    echo "release archive contains an unsafe member type or inconsistent listing" >&2
    exit 1
fi > "$archive_regular_listing"

LC_ALL=C "$tar_bin" -xOzf "$temp_dir/$archive_name" \
    -- "$manifest_entry" > "$source_manifest"
source_version=$(
    awk '
        /^\[package\][[:space:]]*$/ { in_package = 1; next }
        in_package && /^\[/ { exit }
        in_package && /^[[:space:]]*version[[:space:]]*=/ {
            value = $0
            sub(/^[^=]*=[[:space:]]*"/, "", value)
            sub(/"[[:space:]]*(#.*)?$/, "", value)
            print value
            exit
        }
    ' "$source_manifest"
)
source_name=$(
    awk '
        /^\[package\][[:space:]]*$/ { in_package = 1; next }
        in_package && /^\[/ { exit }
        in_package && /^[[:space:]]*name[[:space:]]*=/ {
            value = $0
            sub(/^[^=]*=[[:space:]]*"/, "", value)
            sub(/"[[:space:]]*(#.*)?$/, "", value)
            print value
            exit
        }
    ' "$source_manifest"
)
if [[ "$source_name" != "mitsuro" ]]; then
    echo "release tag v$version contains CLI package ${source_name:-<missing>}, not mitsuro" >&2
    exit 1
fi
if [[ "$source_version" != "$version" ]]; then
    echo "release tag v$version contains mitsuro package version ${source_version:-<missing>}" >&2
    exit 1
fi

for required_entry in \
    "${expected_prefix}Cargo.toml" \
    "${expected_prefix}crates/mitsuro-cli/Cargo.toml" \
    "${expected_prefix}crates/mitsuro-cli/src/bin/krusty.rs" \
    "${expected_prefix}crates/mitsuro-hive/Cargo.toml" \
    "${expected_prefix}crates/mitsuro-hive/src/bin/krusty-mako.rs" \
    "${expected_prefix}deploy/systemd/mitsuro-hive.service" \
    "${expected_prefix}deploy/systemd/mitsuro-hive.socket" \
    "${expected_prefix}deploy/systemd/mitsuro-serve.service"; do
    if ! grep -Fqx "$required_entry" "$archive_regular_listing"; then
        echo "release archive is missing required canonical bridge file: $required_entry" >&2
        exit 1
    fi
done

if command -v sha256sum >/dev/null 2>&1; then
    checksum=$(sha256sum "$temp_dir/$archive_name" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    checksum=$(shasum -a 256 "$temp_dir/$archive_name" | awk '{print $1}')
else
    echo "sha256sum or shasum is required" >&2
    exit 1
fi

update_pkgbuild() {
    local temp_file
    temp_file=$(mktemp "$script_dir/.aur-update.XXXXXX")
    awk \
        -v version="$version" \
        -v checksum="$checksum" \
        -v archive_name="$archive_name" \
        -v archive_url="$archive_url" '
        /^pkgver=/ {
            print "pkgver=" version
            next
        }
        /^pkgrel=/ {
            print "pkgrel=1"
            next
        }
        /^source=/ {
            print "source=(\047" archive_name "::" archive_url "\047)"
            next
        }
        /^sha256sums=/ {
            print "sha256sums=(\047" checksum "\047)"
            next
        }
        { print }
    ' "$pkgbuild" > "$temp_file"
    chmod 0644 "$temp_file"
    mv -- "$temp_file" "$pkgbuild"
}

update_pkgbuild

grep -Fqx "pkgver=$version" "$pkgbuild"
grep -Fqx "pkgrel=1" "$pkgbuild"
grep -Fqx "source=('$archive_name::$archive_url')" "$pkgbuild"
grep -Fqx "sha256sums=('$checksum')" "$pkgbuild"

generated_srcinfo=$(mktemp "$script_dir/.srcinfo-check.XXXXXX")
trap 'rm -rf -- "$temp_dir" "$generated_srcinfo"' EXIT
(
    cd -- "$script_dir"
    makepkg --printsrcinfo
) > "$generated_srcinfo"
chmod 0644 "$generated_srcinfo"
mv -- "$generated_srcinfo" "$srcinfo"
grep -Fqx $'\tpkgver = '"$version" "$srcinfo"
grep -Fqx $'\tpkgrel = 1' "$srcinfo"
grep -Fqx $'\tsource = '"$archive_name::$archive_url" "$srcinfo"
grep -Fqx $'\tsha256sums = '"$checksum" "$srcinfo"

echo "Updated AUR metadata for v$version"
echo "SHA-256: $checksum"
echo "Verify with: makepkg --verifysource -f"
