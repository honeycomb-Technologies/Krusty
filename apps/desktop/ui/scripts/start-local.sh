#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
mkdir -p /tmp/mitsuro-desktop-runtime
if ! curl -sf http://127.0.0.1:3000/health >/dev/null; then
  if [ -x "$ROOT/target/debug/mitsuro-hive" ]; then
    nohup "$ROOT/target/debug/mitsuro-hive" >/tmp/mitsuro-desktop-runtime/hive.log 2>&1 &
    echo $! >/tmp/mitsuro-desktop-runtime/hive.pid || true
    sleep 1
  fi
  nohup "$ROOT/target/debug/mitsuro" serve --port 3000 >/tmp/mitsuro-desktop-runtime/server.log 2>&1 &
  echo $! >/tmp/mitsuro-desktop-runtime/server.pid
  for i in $(seq 1 30); do
    curl -sf http://127.0.0.1:3000/health >/dev/null && break
    sleep 0.4
  done
fi
cd "$ROOT/apps/desktop/ui"
if [ ! -e node_modules ]; then
  ln -sfn /Users/Jacob/Documents/Mitsuro/apps/mobile/node_modules node_modules || true
fi
exec bunx expo start --web --port 5180
