#!/bin/sh
# Copy cargo/mitsuro's SemVer into Expo marketing version fields.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cli_toml="$root/crates/mitsuro-cli/Cargo.toml"
app_json="$root/apps/mobile/app.json"
mobile_pkg="$root/apps/mobile/package.json"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$cli_toml" | head -n 1)
if [ -z "$version" ]; then
  echo "Could not read version from $cli_toml" >&2
  exit 1
fi

python3 - "$version" "$app_json" "$mobile_pkg" <<'PY'
import json
import sys
from pathlib import Path

version, app_path, pkg_path = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])

app = json.loads(app_path.read_text())
app.setdefault("expo", {})["version"] = version
app_path.write_text(json.dumps(app, indent=2) + "\n")

pkg = json.loads(pkg_path.read_text())
pkg["version"] = version
pkg_path.write_text(json.dumps(pkg, indent=2) + "\n")
print(f"Synced Expo marketing version to {version}")
PY
