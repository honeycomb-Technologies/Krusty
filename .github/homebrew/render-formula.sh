#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 <protected-release-tag> <artifact-directory> <output-formula>" >&2
    echo "example: $0 v0.8.0 artifacts artifacts/homebrew/mitsuro.rb" >&2
}

if [[ $# -ne 3 ]]; then
    usage
    exit 2
fi

tag_name=$1
artifact_dir=$2
output_formula=$3

if [[ ! "$tag_name" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
    echo "invalid release tag: $tag_name" >&2
    exit 2
fi

if [[ ! -d "$artifact_dir" ]]; then
    echo "artifact directory does not exist: $artifact_dir" >&2
    exit 1
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)
template="$script_dir/mitsuro.rb"
version=${tag_name#v}

if [[ ! -f "$template" ]]; then
    echo "Homebrew formula template does not exist: $template" >&2
    exit 1
fi

package_version=$(
    cargo metadata \
        --locked \
        --no-deps \
        --format-version 1 \
        --manifest-path "$repo_root/Cargo.toml" |
        ruby -rjson -e '
            packages = JSON.parse(STDIN.read).fetch("packages")
            matches = packages.select { |package| package.fetch("name") == "mitsuro" }
            abort "expected exactly one mitsuro package in Cargo metadata" unless matches.length == 1
            puts matches.first.fetch("version")
        '
)
if [[ "$package_version" != "$version" ]]; then
    echo "release tag $tag_name does not match mitsuro package version $package_version" >&2
    exit 1
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "sha256sum or shasum is required" >&2
        return 1
    fi
}

checksum_for_artifact() {
    local target=$1
    local extension=$2
    local archive_name="mitsuro-$target.$extension"
    local checksum_name="$archive_name.sha256"
    local archive_path=""
    local checksum_path=""
    local archive_count=0
    local checksum_count=0
    local candidate

    while IFS= read -r candidate; do
        archive_path=$candidate
        archive_count=$((archive_count + 1))
    done < <(find "$artifact_dir" -type f -name "$archive_name" -print)

    while IFS= read -r candidate; do
        checksum_path=$candidate
        checksum_count=$((checksum_count + 1))
    done < <(find "$artifact_dir" -type f -name "$checksum_name" -print)

    if [[ $archive_count -ne 1 ]]; then
        echo "expected exactly one $archive_name artifact, found $archive_count" >&2
        return 1
    fi
    if [[ $checksum_count -ne 1 ]]; then
        echo "expected exactly one $checksum_name artifact, found $checksum_count" >&2
        return 1
    fi

    local manifest_line_count
    manifest_line_count=$(awk 'NF { count++ } END { print count + 0 }' "$checksum_path")
    if [[ "$manifest_line_count" -ne 1 ]]; then
        echo "checksum manifest must contain exactly one non-empty line: $checksum_path" >&2
        return 1
    fi

    local expected_checksum recorded_name extra
    read -r expected_checksum recorded_name extra < "$checksum_path"
    recorded_name=${recorded_name#\*}
    expected_checksum=$(printf '%s' "$expected_checksum" | tr '[:upper:]' '[:lower:]')

    if [[ -n "${extra:-}" || ! "$expected_checksum" =~ ^[0-9a-f]{64}$ ]]; then
        echo "malformed SHA-256 manifest: $checksum_path" >&2
        return 1
    fi
    if [[ "$recorded_name" != "$archive_name" ]]; then
        echo "checksum manifest names $recorded_name, expected $archive_name" >&2
        return 1
    fi

    local actual_checksum
    actual_checksum=$(sha256_file "$archive_path")
    if [[ "$actual_checksum" != "$expected_checksum" ]]; then
        echo "SHA-256 mismatch for $archive_name" >&2
        return 1
    fi

    printf '%s\n' "$expected_checksum"
}

macos_arm64_sha=$(checksum_for_artifact aarch64-apple-darwin tar.gz)
macos_x64_sha=$(checksum_for_artifact x86_64-apple-darwin tar.gz)
linux_arm64_sha=$(checksum_for_artifact aarch64-unknown-linux-gnu tar.gz)
linux_x64_sha=$(checksum_for_artifact x86_64-unknown-linux-gnu tar.gz)
# The Windows archive is not referenced by Homebrew, but it is part of the same
# protected release and must pass the identical strict manifest/digest gate.
checksum_for_artifact x86_64-pc-windows-msvc zip >/dev/null

output_dir=$(dirname -- "$output_formula")
mkdir -p -- "$output_dir"
temporary_formula=$(mktemp "$output_dir/.mitsuro-formula.XXXXXX")
trap 'rm -f -- "$temporary_formula"' EXIT

awk \
    -v version="$version" \
    -v macos_arm64_sha="$macos_arm64_sha" \
    -v macos_x64_sha="$macos_x64_sha" \
    -v linux_arm64_sha="$linux_arm64_sha" \
    -v linux_x64_sha="$linux_x64_sha" '
    {
        gsub(/VERSION_PLACEHOLDER/, version)
        gsub(/SHA256_MACOS_ARM64/, macos_arm64_sha)
        gsub(/SHA256_MACOS_X64/, macos_x64_sha)
        gsub(/SHA256_LINUX_ARM64/, linux_arm64_sha)
        gsub(/SHA256_LINUX_X64/, linux_x64_sha)
        print
    }
' "$template" > "$temporary_formula"

if grep -Eq '(VERSION_PLACEHOLDER|SHA256_[A-Z0-9_]+)' "$temporary_formula"; then
    echo "rendered formula still contains template tokens" >&2
    exit 1
fi

grep -Fqx "  version \"$version\"" "$temporary_formula"
for target in \
    aarch64-apple-darwin \
    x86_64-apple-darwin \
    aarch64-unknown-linux-gnu \
    x86_64-unknown-linux-gnu; do
    grep -Fq "/releases/download/$tag_name/mitsuro-$target.tar.gz\"" "$temporary_formula"
done

ruby -c "$temporary_formula" >/dev/null
chmod 0644 "$temporary_formula"
mv -- "$temporary_formula" "$output_formula"
trap - EXIT

echo "Rendered Homebrew formula for $tag_name (mitsuro $package_version): $output_formula"
