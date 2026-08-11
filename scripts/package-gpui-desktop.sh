#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package-gpui-desktop.sh <release-binary> <output-directory>

Builds installable Linux .deb and .rpm packages for the native GPUI desktop.
The release binary must already exist and be executable.
EOF
}

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
binary_path=$(realpath -- "$1")
output_dir=$2
manifest="$repo_root/apps/desktop/gpui/Cargo.toml"
desktop_file="$repo_root/apps/desktop/gpui/packaging/io.mitsuro.desktop.desktop"
metainfo_file="$repo_root/apps/desktop/gpui/packaging/io.mitsuro.desktop.metainfo.xml"
icon_file="$repo_root/apps/desktop/shell/src-tauri/icons/icon-256.png"
asset_dir="$repo_root/apps/desktop/gpui/assets"

if [[ ! -f "$binary_path" || ! -x "$binary_path" ]]; then
  printf 'Release binary is missing or not executable: %s\n' "$binary_path" >&2
  exit 1
fi

for required_file in "$manifest" "$desktop_file" "$metainfo_file" "$icon_file"; do
  if [[ ! -f "$required_file" ]]; then
    printf 'Required packaging input is missing: %s\n' "$required_file" >&2
    exit 1
  fi
done
if [[ ! -d "$asset_dir/icons" ]]; then
  printf 'Required GPUI asset directory is missing: %s\n' "$asset_dir" >&2
  exit 1
fi

version=$(awk '
  $0 == "[package]" { in_package = 1; next }
  /^\[/ { in_package = 0 }
  in_package && $1 == "version" {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' "$manifest")
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([._+-][0-9A-Za-z.-]+)?$ ]]; then
  printf 'Could not read a package-safe version from %s: %s\n' "$manifest" "$version" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64|amd64)
    deb_arch=amd64
    rpm_arch=x86_64
    ;;
  aarch64|arm64)
    deb_arch=arm64
    rpm_arch=aarch64
    ;;
  *)
    printf 'Unsupported Linux package architecture: %s\n' "$(uname -m)" >&2
    exit 1
    ;;
esac

formats=${MITSURO_GPUI_PACKAGE_FORMATS:-deb,rpm}
case ",$formats," in
  *,deb,*) build_deb=true ;;
  *) build_deb=false ;;
esac
case ",$formats," in
  *,rpm,*) build_rpm=true ;;
  *) build_rpm=false ;;
esac
if [[ "$build_deb" != true && "$build_rpm" != true ]]; then
  printf 'MITSURO_GPUI_PACKAGE_FORMATS must include deb, rpm, or both.\n' >&2
  exit 1
fi
if [[ "$build_rpm" == true ]] && ! command -v rpmbuild >/dev/null 2>&1; then
  printf 'rpmbuild is required to build the RPM package.\n' >&2
  exit 1
fi

mkdir -p -- "$output_dir"
output_dir=$(realpath -- "$output_dir")
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/mitsuro-gpui-package.XXXXXXXX")
cleanup() {
  rm -rf -- "$work_dir"
}
trap cleanup EXIT

stage_payload() {
  local root=$1
  install -Dm755 "$binary_path" "$root/usr/bin/mitsuro-desktop"
  install -Dm644 "$desktop_file" \
    "$root/usr/share/applications/io.mitsuro.desktop.desktop"
  install -Dm644 "$metainfo_file" \
    "$root/usr/share/metainfo/io.mitsuro.desktop.metainfo.xml"
  install -Dm644 "$icon_file" \
    "$root/usr/share/icons/hicolor/256x256/apps/io.mitsuro.desktop.png"
  install -Dm644 "$repo_root/LICENSE" "$root/usr/share/licenses/mitsuro-gpui-desktop/LICENSE"
  mkdir -p -- "$root/usr/share/mitsuro-gpui-desktop"
  cp -R -- "$asset_dir" "$root/usr/share/mitsuro-gpui-desktop/assets"
  find "$root/usr/share/mitsuro-gpui-desktop/assets" -type d -exec chmod 755 {} +
  find "$root/usr/share/mitsuro-gpui-desktop/assets" -type f -exec chmod 644 {} +
}

if [[ "$build_deb" == true ]]; then
  deb_root="$work_dir/deb-root"
  stage_payload "$deb_root"
  mkdir -p -- "$deb_root/DEBIAN"
  installed_kib=$(du -sk "$deb_root/usr" | awk '{print $1}')
  cat >"$deb_root/DEBIAN/control" <<EOF
Package: mitsuro-gpui-desktop
Version: $version
Architecture: $deb_arch
Maintainer: Honeycomb Technologies
Installed-Size: $installed_kib
Depends: libgtk-3-0, libwebkit2gtk-4.1-0, libxkbcommon0, libxkbcommon-x11-0, xdg-utils
Section: utils
Priority: optional
Homepage: https://github.com/honeycomb-Technologies/Mitsuro
Description: Native GPUI client for Mitsuro and Codex app-server
 Mitsuro Desktop provides a native dual-backend workspace for Mitsuro HTTP/SSE
 and a managed Codex app-server process.
EOF
  deb_output="$output_dir/mitsuro-gpui-desktop_${version}_${deb_arch}.deb"
  if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb --build --root-owner-group "$deb_root" "$deb_output" >/dev/null
  else
    if ! command -v ar >/dev/null 2>&1 || ! command -v tar >/dev/null 2>&1; then
      printf 'dpkg-deb, or both ar and tar, are required for Debian packaging.\n' >&2
      exit 1
    fi
    deb_archive="$work_dir/deb-archive"
    mkdir -p -- "$deb_archive"
    printf '2.0\n' >"$deb_archive/debian-binary"
    tar --sort=name --owner=0 --group=0 --numeric-owner \
      -C "$deb_root/DEBIAN" -czf "$deb_archive/control.tar.gz" .
    tar --sort=name --owner=0 --group=0 --numeric-owner \
      -C "$deb_root" -czf "$deb_archive/data.tar.gz" ./usr
    rm -f -- "$deb_output"
    (
      cd -- "$deb_archive"
      ar qc "$deb_output" debian-binary control.tar.gz data.tar.gz
    )
  fi
  printf 'Built %s\n' "$deb_output"
fi

if [[ "$build_rpm" == true ]]; then
  rpm_top="$work_dir/rpmbuild"
  mkdir -p -- "$rpm_top"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
  cp -- "$binary_path" "$rpm_top/SOURCES/mitsuro-desktop"
  cp -- "$desktop_file" "$rpm_top/SOURCES/io.mitsuro.desktop.desktop"
  cp -- "$metainfo_file" "$rpm_top/SOURCES/io.mitsuro.desktop.metainfo.xml"
  cp -- "$icon_file" "$rpm_top/SOURCES/io.mitsuro.desktop.png"
  cp -- "$repo_root/LICENSE" "$rpm_top/SOURCES/LICENSE"
  tar -C "$repo_root/apps/desktop/gpui" -czf "$rpm_top/SOURCES/gpui-assets.tar.gz" assets
  cat >"$rpm_top/SPECS/mitsuro-gpui-desktop.spec" <<EOF
Name: mitsuro-gpui-desktop
Version: $version
Release: 1%{?dist}
Summary: Native GPUI client for Mitsuro and Codex app-server
License: MIT
URL: https://github.com/honeycomb-Technologies/Mitsuro
BuildArch: $rpm_arch
Source0: mitsuro-desktop
Source1: io.mitsuro.desktop.desktop
Source2: io.mitsuro.desktop.metainfo.xml
Source3: io.mitsuro.desktop.png
Source4: LICENSE
Source5: gpui-assets.tar.gz
Requires: gtk3
Requires: webkit2gtk4.1
Requires: libxkbcommon
Requires: libxkbcommon-x11
Requires: xdg-utils

%description
Mitsuro Desktop provides a native dual-backend workspace for Mitsuro HTTP/SSE
and a managed Codex app-server process.

%prep

%build

%install
install -Dm755 %{SOURCE0} %{buildroot}/usr/bin/mitsuro-desktop
install -Dm644 %{SOURCE1} %{buildroot}/usr/share/applications/io.mitsuro.desktop.desktop
install -Dm644 %{SOURCE2} %{buildroot}/usr/share/metainfo/io.mitsuro.desktop.metainfo.xml
install -Dm644 %{SOURCE3} %{buildroot}/usr/share/icons/hicolor/256x256/apps/io.mitsuro.desktop.png
install -Dm644 %{SOURCE4} %{buildroot}/usr/share/licenses/%{name}/LICENSE
mkdir -p %{buildroot}/usr/share/mitsuro-gpui-desktop
tar -xzf %{SOURCE5} -C %{buildroot}/usr/share/mitsuro-gpui-desktop

%files
/usr/bin/mitsuro-desktop
/usr/share/applications/io.mitsuro.desktop.desktop
/usr/share/metainfo/io.mitsuro.desktop.metainfo.xml
/usr/share/icons/hicolor/256x256/apps/io.mitsuro.desktop.png
/usr/share/licenses/%{name}/LICENSE
/usr/share/mitsuro-gpui-desktop/assets

%changelog
* Tue Aug 11 2026 Honeycomb Technologies - $version-1
- Package the native Mitsuro GPUI desktop.
EOF
  rpmbuild -bb --quiet --define "_topdir $rpm_top" \
    "$rpm_top/SPECS/mitsuro-gpui-desktop.spec"
  rpm_output=$(find "$rpm_top/RPMS/$rpm_arch" -maxdepth 1 -type f -name '*.rpm' -print -quit)
  if [[ -z "$rpm_output" ]]; then
    printf 'rpmbuild did not produce an RPM for %s.\n' "$rpm_arch" >&2
    exit 1
  fi
  cp -- "$rpm_output" "$output_dir/"
  printf 'Built %s/%s\n' "$output_dir" "${rpm_output##*/}"
fi
