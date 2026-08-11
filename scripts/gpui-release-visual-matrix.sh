#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/gpui-release-visual-matrix.sh <desktop-binary> <output-directory>

Launches the native GPUI desktop in every top-level product mode against the real
Codex stdio and Mitsuro HTTP backends, then captures each mapped Wayland window.
The run is read-only: provider turns are disabled and fixture mode is forbidden.

Environment overrides:
  MITSURO_SERVER_URL       Mitsuro HTTP endpoint (default: http://127.0.0.1:3000)
  MITSURO_VISUAL_BACKENDS  Comma-separated backend ids
  MITSURO_VISUAL_MODES     Comma-separated MITSURO_START_MODE values
EOF
}

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

for command_name in grim hyprctl jq rg; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf '%s is required for the Wayland visual matrix.\n' "$command_name" >&2
    exit 1
  fi
done

binary_path=$(realpath -- "$1")
output_dir=$2
if [[ ! -f "$binary_path" || ! -x "$binary_path" ]]; then
  printf 'Desktop binary is missing or not executable: %s\n' "$binary_path" >&2
  exit 1
fi

mkdir -p -- "$output_dir"
output_dir=$(realpath -- "$output_dir")
server_url=${MITSURO_SERVER_URL:-http://127.0.0.1:3000}
backend_csv=${MITSURO_VISUAL_BACKENDS:-codex-stdio,mitsuro-http}
mode_csv=${MITSURO_VISUAL_MODES:-chat,codex,thread-open,work,atlas,terminal,files,computer,extensions,settings,pull-requests,sites,scheduled}
IFS=',' read -r -a backends <<<"$backend_csv"
IFS=',' read -r -a modes <<<"$mode_csv"

active_pid=
stop_active_app() {
  if [[ -n "$active_pid" ]] && kill -0 "$active_pid" 2>/dev/null; then
    kill -TERM "$active_pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      if ! kill -0 "$active_pid" 2>/dev/null; then
        break
      fi
      sleep 0.1
    done
    if kill -0 "$active_pid" 2>/dev/null; then
      kill -KILL "$active_pid" 2>/dev/null || true
    fi
    wait "$active_pid" 2>/dev/null || true
  fi
  active_pid=
}
trap stop_active_app EXIT
trap 'exit 130' INT TERM

window_for_pid() {
  local pid=$1
  hyprctl clients -j 2>/dev/null \
    | jq -c --argjson pid "$pid" \
      '.[] | select(.pid == $pid and .class == "io.mitsuro.desktop" and .mapped == true)' \
    | head -n 1
}

for backend in "${backends[@]}"; do
  case "$backend" in
    codex-stdio|mitsuro-http) ;;
    *)
      printf 'Unsupported visual-matrix backend: %s\n' "$backend" >&2
      exit 1
      ;;
  esac

  backend_slug=${backend//-/_}
  for mode in "${modes[@]}"; do
    mode_slug=${mode//-/_}
    log_path="$output_dir/${backend_slug}-${mode_slug}.log"
    image_path="$output_dir/${backend_slug}-${mode_slug}.png"
    printf 'Capturing backend=%s mode=%s\n' "$backend" "$mode"

    MITSURO_BACKEND="$backend" \
      MITSURO_SERVER_URL="$server_url" \
      MITSURO_NO_LIVE_TURN=1 \
      MITSURO_START_MODE="$mode" \
      "$binary_path" >"$log_path" 2>&1 &
    active_pid=$!

    window_json=
    for _ in $(seq 1 150); do
      if ! kill -0 "$active_pid" 2>/dev/null; then
        printf 'Desktop exited before mapping for backend=%s mode=%s\n' "$backend" "$mode" >&2
        sed -n '1,120p' "$log_path" >&2
        exit 1
      fi
      window_json=$(window_for_pid "$active_pid")
      if [[ -n "$window_json" ]]; then
        break
      fi
      sleep 0.1
    done
    if [[ -z "$window_json" ]]; then
      printf 'No mapped GPUI window for backend=%s mode=%s\n' "$backend" "$mode" >&2
      exit 1
    fi

    connected_pattern="Connected backend=${backend} "
    for _ in $(seq 1 200); do
      if rg -q --fixed-strings "$connected_pattern" "$log_path"; then
        break
      fi
      if ! kill -0 "$active_pid" 2>/dev/null; then
        printf 'Desktop exited before connecting for backend=%s mode=%s\n' "$backend" "$mode" >&2
        sed -n '1,120p' "$log_path" >&2
        exit 1
      fi
      sleep 0.1
    done
    if ! rg -q --fixed-strings "$connected_pattern" "$log_path"; then
      printf 'Desktop did not connect for backend=%s mode=%s\n' "$backend" "$mode" >&2
      sed -n '1,120p' "$log_path" >&2
      exit 1
    fi

    # Give post-connect catalog and selected-thread hydration one bounded paint cycle.
    sleep 1
    window_json=$(window_for_pid "$active_pid")
    if [[ -z "$window_json" ]]; then
      printf 'Mapped GPUI window disappeared for backend=%s mode=%s\n' "$backend" "$mode" >&2
      exit 1
    fi
    window_at=$(jq -r '.at | "\(.[0]),\(.[1])"' <<<"$window_json")
    window_size=$(jq -r '.size | "\(.[0])x\(.[1])"' <<<"$window_json")
    grim -g "$window_at $window_size" "$image_path"
    if [[ ! -s "$image_path" ]]; then
      printf 'Visual capture is empty: %s\n' "$image_path" >&2
      exit 1
    fi

    stop_active_app
  done
done

printf 'Captured %s release visuals in %s\n' "$(find "$output_dir" -maxdepth 1 -type f -name '*.png' | wc -l)" "$output_dir"
