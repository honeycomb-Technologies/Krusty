#!/bin/sh
set -eu

# Stage the pinned agent-browser native runtime for local builds and release archives.
# Chromium itself is discovered from the host; run `target/atlas/agent-browser install`
# explicitly when a machine does not already provide Chrome or Chromium.

ATLAS_VERSION=0.34.0
ATLAS_ARCHIVE_SHA256=a4744fb189e598467abcfb3acdde07118d9e5cb43dc3b31727f869af4eb9d598
ATLAS_DESTINATION=${ATLAS_DESTINATION:-target/atlas/agent-browser}

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) source_name=agent-browser-linux-x64 ;;
    Linux-aarch64|Linux-arm64) source_name=agent-browser-linux-arm64 ;;
    Darwin-x86_64) source_name=agent-browser-darwin-x64 ;;
    Darwin-arm64) source_name=agent-browser-darwin-arm64 ;;
    MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64)
        source_name=agent-browser-win32-x64.exe
        ;;
    *)
        echo "Atlas runtime is not packaged for $(uname -s)-$(uname -m)." >&2
        exit 1
        ;;
esac

temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

(cd "$temporary_dir" && npm pack --silent "agent-browser@$ATLAS_VERSION" >/dev/null)
archive="$temporary_dir/agent-browser-$ATLAS_VERSION.tgz"

if command -v sha256sum >/dev/null 2>&1; then
    actual_sha256=$(sha256sum "$archive" | awk '{print $1}')
else
    actual_sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
fi
if [ "$actual_sha256" != "$ATLAS_ARCHIVE_SHA256" ]; then
    echo "Atlas runtime archive checksum mismatch." >&2
    exit 1
fi

tar xzf "$archive" -C "$temporary_dir"
source_path="$temporary_dir/package/bin/$source_name"
test -f "$source_path"
mkdir -p "$(dirname "$ATLAS_DESTINATION")"
cp "$source_path" "$ATLAS_DESTINATION"
chmod 0555 "$ATLAS_DESTINATION"
"$ATLAS_DESTINATION" --version
