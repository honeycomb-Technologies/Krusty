#!/usr/bin/env bash
set -euo pipefail

HOST="${KRUSTY_MACBOOK_TAILSCALE_HOST:-${KRUSTY_MAC_HOST:-haleys-macbook-air}}"
SSH_USER="${KRUSTY_MACBOOK_SSH_USER:-${KRUSTY_MAC_USER:-}}"
SSH_KEY="${KRUSTY_MACBOOK_SSH_KEY:-${KRUSTY_MAC_SSH_KEY:-}}"
SSH_TARGET="${SSH_USER:+$SSH_USER@}$HOST"
SSH_OPTIONS=(-o BatchMode=yes -o ConnectTimeout=5)
if [[ -n "$SSH_KEY" ]]; then
	SSH_OPTIONS+=(-i "$SSH_KEY" -o IdentitiesOnly=yes)
fi

printf '== Krusty MacBook reachability check ==\n'
printf 'tailscale host: %s\n' "$HOST"

if ! command -v tailscale >/dev/null 2>&1; then
	printf 'tailscale: not installed on this machine\n'
	exit 2
fi

tailscale status | grep -E "(^|[[:space:]])$HOST([[:space:]]|$)" || true

if tailscale ping --timeout=3s --c 2 "$HOST"; then
	printf 'tailscale ping: ok\n'
else
	printf 'tailscale ping: no reply. MacBook is probably asleep/offline or Tailscale is not active.\n'
	exit 1
fi

if [[ -z "$SSH_USER" ]]; then
	printf 'ssh: skipped. Set KRUSTY_MACBOOK_SSH_USER=<mac-user> or KRUSTY_MAC_USER=<mac-user> to test SSH.\n'
	exit 0
fi

REMOTE_CHECK='set -e
printf "uname: "; uname -a
printf "xcodebuild: "; command -v xcodebuild || true
printf "swift: "; command -v swift || true
printf "cargo: "; command -v cargo || true
printf "rust ios targets:\n"; rustup target list --installed 2>/dev/null | grep -E "aarch64-apple-ios|x86_64-apple-ios|aarch64-apple-ios-sim" || true
'

if ssh "${SSH_OPTIONS[@]}" "$SSH_TARGET" "$REMOTE_CHECK"; then
	printf 'ssh: ok\n'
else
	printf 'ssh: failed. Enable Remote Login on macOS and ensure this machine has an authorized key.\n'
	exit 1
fi
