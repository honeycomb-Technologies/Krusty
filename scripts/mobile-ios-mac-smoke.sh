#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IOS_DIR="$ROOT/apps/ios"
SHELL_DIR="$IOS_DIR/KrustyMobileShell"
APP_DIR="$IOS_DIR/KrustyMobileApp"
CHECK_GPUI_IOS="${MOBILE_IOS_CHECK_GPUI:-0}"

printf '== Krusty iOS/Mac smoke ==\n'
printf 'root: %s\n' "$ROOT"
xcodebuild -version
swift --version | head -1
cargo --version
rustc --version

printf '\n== Swift shell package: generic iOS ==\n'
(
	cd "$SHELL_DIR"
	xcodebuild \
		-scheme KrustyMobileShell \
		-destination 'generic/platform=iOS' \
		-configuration Debug \
		build
)

printf '\n== Native iOS app target: simulator ==\n'
(
	cd "$APP_DIR"
	xcodebuild \
		-project KrustyMobileApp.xcodeproj \
		-scheme KrustyMobileApp \
		-destination 'generic/platform=iOS Simulator' \
		-configuration Debug \
		CODE_SIGNING_ALLOWED=NO \
		build
)

printf '\n== Native iOS app target: device compile/no signing ==\n'
(
	cd "$APP_DIR"
	xcodebuild \
		-project KrustyMobileApp.xcodeproj \
		-scheme KrustyMobileApp \
		-destination 'generic/platform=iOS' \
		-configuration Debug \
		CODE_SIGNING_ALLOWED=NO \
		build
)

printf '\n== Rust mobile client/state: iOS simulator ==\n'
(
	cd "$ROOT"
	cargo check -p krusty-client -p krusty-client-state --target aarch64-apple-ios-sim
)

if [[ "$CHECK_GPUI_IOS" == "1" ]]; then
	printf '\n== Optional GPUI mobile crate iOS gate ==\n'
	(
		cd "$ROOT"
		cargo check -p krusty-mobile --target aarch64-apple-ios-sim
	)
else
	printf '\nGPUI iOS crate check skipped. Set MOBILE_IOS_CHECK_GPUI=1 to run the known hard gate.\n'
fi

printf '\nKrusty iOS/Mac smoke passed\n'
