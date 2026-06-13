#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-${KRUSTY_IOS_RUST_TARGET:-aarch64-apple-ios-sim}}"
PROFILE="${KRUSTY_IOS_RUST_PROFILE:-debug}"
OUT_DIR="$ROOT/target/krusty-mobile-ios/$TARGET/$PROFILE"

PROFILE_ARGS=()
PROFILE_DIR="debug"
if [[ "$PROFILE" == "release" ]]; then
	PROFILE_ARGS=(--release)
	PROFILE_DIR="release"
elif [[ "$PROFILE" != "debug" ]]; then
	printf 'Unsupported KRUSTY_IOS_RUST_PROFILE=%s (expected debug or release)\n' "$PROFILE" >&2
	exit 2
fi

printf '== Build krusty-mobile Rust staticlib ==\n'
printf 'target: %s\n' "$TARGET"
printf 'profile: %s\n' "$PROFILE"

rustup target add "$TARGET"
(
	cd "$ROOT"
	cargo rustc \
		-p krusty-mobile \
		--target "$TARGET" \
		--lib \
		"${PROFILE_ARGS[@]}" \
		-- \
		--crate-type=staticlib
)

mkdir -p "$OUT_DIR"
LIB_PATH="$(find "$ROOT/target/$TARGET/$PROFILE_DIR/deps" -maxdepth 1 -type f -name 'libkrusty_mobile-*.a' -print | sort | tail -1)"
if [[ -z "$LIB_PATH" ]]; then
	printf 'Failed to locate libkrusty_mobile static archive under target/%s/%s/deps\n' "$TARGET" "$PROFILE_DIR" >&2
	exit 1
fi
cp "$LIB_PATH" "$OUT_DIR/libkrusty_mobile.a"

printf '\nstaticlib: %s\n' "$OUT_DIR/libkrusty_mobile.a"
printf 'Use Xcode -force_load with this archive so dlsym-visible Rust entrypoints are retained.\n'
