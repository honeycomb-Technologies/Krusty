#!/bin/sh
set -e

# Mitsuro installer (compatibility package and binary: krusty)
# Usage: curl -fsSLO https://raw.githubusercontent.com/honeycomb-Technologies/Mitsuro/main/install.sh && sh install.sh
# Validation: sh install.sh --self-test

REPO="honeycomb-Technologies/Mitsuro"
BINARY="krusty"
DAEMON_BINARY="krusty-mako"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
SYSTEMD_UNITS="krusty-mako.socket krusty-mako.service krusty-serve.service"
ACTIVATION_HEALTH_ATTEMPTS=20
ACTIVATION_STABLE_PASSES=2

TMP_DIR=""
INSTALL_LOCK=""
LOCK_HELD=false
RELEASE_STAGE=""
ATOMIC_TEMP=""
ACTIVATION_BACKUP=""
ACTIVATION_IN_PROGRESS=false
ACTIVATION_ROLLING_BACK=false
SELF_TEST_FAIL_POINT=""
SELF_TEST_SIGNAL_POINT=""
SELF_TEST_CAPTURE_SIGNAL=false

arm_signal_traps() {
    trap 'activation_signal HUP' HUP
    trap 'activation_signal INT' INT
    trap 'activation_signal TERM' TERM
}

remove_writable_tree() {
    writable_tree=$1
    if [ -n "$writable_tree" ] && [ -d "$writable_tree" ]; then
        chmod -R u+w "$writable_tree" 2>/dev/null || true
        rm -rf "$writable_tree"
    fi
}

cleanup_activation_backup() {
    remove_writable_tree "$ACTIVATION_BACKUP"
    ACTIVATION_BACKUP=""
}

cleanup_atomic_temp() {
    if [ -n "$ATOMIC_TEMP" ]; then
        rm -f "$ATOMIC_TEMP" 2>/dev/null || true
    fi
    ATOMIC_TEMP=""
}

cleanup() {
    cleanup_status=$?
    trap - 0
    trap '' HUP INT TERM
    if [ "$ACTIVATION_IN_PROGRESS" = true ] && [ "$ACTIVATION_ROLLING_BACK" != true ]; then
        rollback_release "installer exited during activation" >/dev/null 2>&1 || true
    fi
    cleanup_atomic_temp
    if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
        remove_writable_tree "$TMP_DIR"
    fi
    if [ -n "$RELEASE_STAGE" ] && [ -d "$RELEASE_STAGE" ]; then
        remove_writable_tree "$RELEASE_STAGE"
    fi
    cleanup_activation_backup
    if [ "$LOCK_HELD" = true ] && [ -n "$INSTALL_LOCK" ]; then
        rmdir "$INSTALL_LOCK" 2>/dev/null || true
    fi
    exit "$cleanup_status"
}

trap cleanup 0
arm_signal_traps

activation_signal() {
    activation_signal_name=$1
    trap '' HUP INT TERM
    if [ "$ACTIVATION_IN_PROGRESS" = true ] && [ "$ACTIVATION_ROLLING_BACK" != true ]; then
        rollback_release "interrupted by $activation_signal_name" || true
    fi
    if [ "$SELF_TEST_CAPTURE_SIGNAL" = true ]; then
        arm_signal_traps
        return 1
    fi
    exit 1
}

fail() {
    echo "Error: $*" >&2
    return 1
}

# Detect OS and architecture.
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64) PLATFORM="x86_64-unknown-linux-gnu" ;;
                aarch64|arm64) PLATFORM="aarch64-unknown-linux-gnu" ;;
                *) fail "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            EXT="tar.gz"
            ;;
        Darwin)
            case "$ARCH" in
                x86_64) PLATFORM="x86_64-apple-darwin" ;;
                arm64) PLATFORM="aarch64-apple-darwin" ;;
                *) fail "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            EXT="tar.gz"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            PLATFORM="x86_64-pc-windows-msvc"
            EXT="zip"
            ;;
        *)
            fail "Unsupported OS: $OS"
            exit 1
            ;;
    esac
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | \
        grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/'
}

sha256_file() {
    checksum_path=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$checksum_path" | awk '{print tolower($1)}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$checksum_path" | awk '{print tolower($1)}'
    else
        fail "sha256sum or shasum is required to verify downloads."
        return 1
    fi
}

published_sha256() {
    checksum_file=$1
    checksum_archive=$2
    awk -v expected_name="$checksum_archive" '
        NR == 1 {
            name = $2
            sub(/^\*/, "", name)
            if (length($1) == 64 && $1 !~ /[^0-9A-Fa-f]/ && name == expected_name) {
                digest = tolower($1)
                valid = 1
                next
            }
        }
        { valid = 0; exit 1 }
        END {
            if (NR != 1 || valid != 1) exit 1
            print digest
        }
    ' "$checksum_file"
}

verify_download() {
    verify_archive=$1
    verify_checksum=$2
    verify_name=$3

    if ! EXPECTED_SHA256="$(published_sha256 "$verify_checksum" "$verify_name")"; then
        fail "Release checksum must contain exactly one sha256sum record for $verify_name."
        return 1
    fi
    if ! ARCHIVE_SHA256="$(sha256_file "$verify_archive")"; then
        return 1
    fi
    if [ "$ARCHIVE_SHA256" != "$EXPECTED_SHA256" ]; then
        fail "Checksum verification failed; the downloaded archive may be corrupted."
        return 1
    fi
}

extract_archive() {
    extract_source=$1
    extract_destination=$2
    if ! preflight_archive "$extract_source"; then
        fail "Archive contains an unsafe path or a non-regular payload entry."
        return 1
    fi
    mkdir -p "$extract_destination"
    if [ "$EXT" = "tar.gz" ]; then
        tar xzf "$extract_source" -C "$extract_destination"
    else
        unzip -q "$extract_source" -d "$extract_destination"
    fi
}

safe_member_paths() {
    awk '
        function unsafe(path, count, parts, i) {
            if (path == "" || substr(path, 1, 1) == "/" || path ~ /\\/ || path ~ /^[A-Za-z]:/) return 1
            count = split(path, parts, "/")
            for (i = 1; i <= count; i++) {
                if (parts[i] == "..") return 1
            }
            return 0
        }
        { if (unsafe($0)) exit 1; seen = 1 }
        END { if (!seen) exit 1 }
    '
}

safe_tar_member_types() {
    awk '
        {
            kind = substr($1, 1, 1)
            if (kind != "-" && kind != "d") exit 1
            seen = 1
        }
        END { if (!seen) exit 1 }
    '
}

safe_zip_member_types() {
    expected_zip_entries=$1
    awk -v expected="$expected_zip_entries" '
        {
            kind = substr($1, 1, 1)
            if (kind == "-" || kind == "d") {
                seen = 1
                entries++
            } else if (kind == "l" || kind == "b" || kind == "c" || kind == "p" || kind == "s" || kind == "?") {
                exit 1
            }
        }
        END { if (!seen || entries != expected) exit 1 }
    '
}

preflight_archive() {
    preflight_source=$1
    if [ "$EXT" = "tar.gz" ]; then
        tar tzf "$preflight_source" | safe_member_paths || return 1
        tar tvzf "$preflight_source" | safe_tar_member_types || return 1
    else
        zip_entries=$(unzip -Z1 "$preflight_source" | awk 'END { print NR + 0 }') || return 1
        [ "$zip_entries" -gt 0 ] || return 1
        unzip -Z1 "$preflight_source" | safe_member_paths || return 1
        unzip -Z -l "$preflight_source" | safe_zip_member_types "$zip_entries" || return 1
    fi
}

acquire_install_lock() {
    mkdir -p "$INSTALL_DIR"
    INSTALL_LOCK="$INSTALL_DIR/.krusty-install.lock"
    if ! mkdir "$INSTALL_LOCK" 2>/dev/null; then
        fail "Another Mitsuro install is running (or left $INSTALL_LOCK behind)."
        return 1
    fi
    LOCK_HELD=true
}

# Replace a path with rename(2) semantics where the platform's mv supports
# them. GNU mv uses -T and BSD mv uses -h.
atomic_replace_path() {
    replace_source=$1
    replace_destination=$2
    if mv -f -T "$replace_source" "$replace_destination" 2>/dev/null; then
        return 0
    fi
    if [ ! -e "$replace_source" ] && [ ! -L "$replace_source" ]; then
        return 0
    fi
    if mv -f -h "$replace_source" "$replace_destination" 2>/dev/null; then
        return 0
    fi
    if [ ! -e "$replace_source" ] && [ ! -L "$replace_source" ]; then
        return 0
    fi

    echo "Warning: atomic path replacement is unavailable on this platform." >&2
    rm -f "$replace_destination"
    mv -f "$replace_source" "$replace_destination"
}

atomic_symlink() {
    link_target=$1
    link_path=$2
    link_parent=$(dirname "$link_path")
    link_leaf=$(basename "$link_path")
    link_tmp="$link_parent/.$link_leaf.krusty-new.$$"

    rm -f "$link_tmp"
    ATOMIC_TEMP="$link_tmp"
    if ! ln -s "$link_target" "$link_tmp" || ! atomic_replace_path "$link_tmp" "$link_path"; then
        cleanup_atomic_temp
        return 1
    fi
    ATOMIC_TEMP=""
}

file_mode() {
    mode_path=$1
    if mode_value=$(stat -c '%a' "$mode_path" 2>/dev/null); then
        printf '%s\n' "$mode_value"
    else
        stat -f '%Lp' "$mode_path" 2>/dev/null
    fi
}

regular_file_with_mode() {
    mode_path=$1
    expected_mode=$2
    [ -f "$mode_path" ] && [ ! -L "$mode_path" ] && [ "$(file_mode "$mode_path")" = "$expected_mode" ]
}

same_optional_file() {
    optional_left=$1
    optional_right=$2
    optional_mode=$3
    if [ -f "$optional_left" ] && [ ! -L "$optional_left" ]; then
        regular_file_with_mode "$optional_left" "$optional_mode" && \
            regular_file_with_mode "$optional_right" "$optional_mode" && \
            cmp -s "$optional_left" "$optional_right"
    else
        [ ! -e "$optional_right" ] && [ ! -L "$optional_right" ]
    fi
}

same_release() {
    candidate_release=$1
    existing_release=$2
    [ -d "$existing_release" ] && [ ! -L "$existing_release" ] && \
        [ "$(file_mode "$existing_release")" = "555" ] || return 1
    regular_file_with_mode "$candidate_release/krusty" 555 || return 1
    regular_file_with_mode "$existing_release/krusty" 555 || return 1
    cmp -s "$candidate_release/krusty" "$existing_release/krusty" || return 1
    regular_file_with_mode "$candidate_release/.archive-sha256" 444 || return 1
    regular_file_with_mode "$existing_release/.archive-sha256" 444 || return 1
    cmp -s "$candidate_release/.archive-sha256" "$existing_release/.archive-sha256" || return 1
    same_optional_file "$candidate_release/krusty-mako" "$existing_release/krusty-mako" 555 || return 1
    if [ -d "$candidate_release/systemd" ] || [ -d "$existing_release/systemd" ]; then
        [ -d "$candidate_release/systemd" ] && [ ! -L "$candidate_release/systemd" ] && \
            [ "$(file_mode "$candidate_release/systemd")" = "555" ] || return 1
        [ -d "$existing_release/systemd" ] && [ ! -L "$existing_release/systemd" ] && \
            [ "$(file_mode "$existing_release/systemd")" = "555" ] || return 1
    fi
    for compare_unit in $SYSTEMD_UNITS; do
        same_optional_file "$candidate_release/systemd/$compare_unit" "$existing_release/systemd/$compare_unit" 444 || return 1
    done
}

discard_release_stage() {
    remove_writable_tree "$RELEASE_STAGE"
    RELEASE_STAGE=""
}

stage_unix_release() {
    payload_dir=$1

    if [ ! -f "$payload_dir/$BINARY" ] || [ -L "$payload_dir/$BINARY" ]; then
        fail "The verified archive does not contain a regular $BINARY binary."
        return 1
    fi

    payload_has_daemon=false
    if [ -e "$payload_dir/$DAEMON_BINARY" ] || [ -L "$payload_dir/$DAEMON_BINARY" ]; then
        if [ ! -f "$payload_dir/$DAEMON_BINARY" ] || [ -L "$payload_dir/$DAEMON_BINARY" ]; then
            fail "$DAEMON_BINARY must be a regular file when present."
            return 1
        fi
        payload_has_daemon=true
    fi

    payload_has_systemd=false
    if [ -d "$payload_dir/systemd" ]; then
        payload_has_systemd=true
        for payload_unit in $SYSTEMD_UNITS; do
            if [ ! -f "$payload_dir/systemd/$payload_unit" ] || [ -L "$payload_dir/systemd/$payload_unit" ]; then
                fail "The systemd payload is incomplete: missing regular $payload_unit."
                return 1
            fi
        done
    elif [ -e "$payload_dir/systemd" ] || [ -L "$payload_dir/systemd" ]; then
        fail "The systemd payload must be a directory when present."
        return 1
    fi

    if [ "$payload_has_daemon" != "$payload_has_systemd" ]; then
        fail "A release must ship $DAEMON_BINARY and its complete systemd unit set together."
        return 1
    fi

    safe_version=$(printf '%s' "$VERSION" | sed 's/[^A-Za-z0-9._-]/_/g')
    [ -n "$safe_version" ] || safe_version="release"
    RELEASE_ID="$safe_version-$PLATFORM-$ARCHIVE_SHA256"
    RELEASES_DIR="$INSTALL_DIR/.krusty-releases"
    RELEASE_DIR="$RELEASES_DIR/$RELEASE_ID"
    release_stage="$RELEASES_DIR/.stage-$RELEASE_ID-$$"
    RELEASE_STAGE="$release_stage"

    mkdir -p "$RELEASES_DIR"
    if [ -e "$release_stage" ] || [ -L "$release_stage" ]; then
        fail "Unexpected release staging path already exists: $release_stage"
        return 1
    fi
    mkdir "$release_stage"
    cp "$payload_dir/$BINARY" "$release_stage/$BINARY"
    chmod 0555 "$release_stage/$BINARY"
    if [ "$payload_has_daemon" = true ]; then
        cp "$payload_dir/$DAEMON_BINARY" "$release_stage/$DAEMON_BINARY"
        chmod 0555 "$release_stage/$DAEMON_BINARY"
        mkdir "$release_stage/systemd"
        for payload_unit in $SYSTEMD_UNITS; do
            cp "$payload_dir/systemd/$payload_unit" "$release_stage/systemd/$payload_unit"
            chmod 0444 "$release_stage/systemd/$payload_unit"
        done
        chmod 0555 "$release_stage/systemd"
    fi
    printf '%s\n' "$ARCHIVE_SHA256" > "$release_stage/.archive-sha256"
    chmod 0444 "$release_stage/.archive-sha256"

    if [ -d "$RELEASE_DIR" ] && [ ! -L "$RELEASE_DIR" ]; then
        if ! same_release "$release_stage" "$RELEASE_DIR"; then
            discard_release_stage
            fail "Immutable release $RELEASE_ID already exists with different contents."
            return 1
        fi
        discard_release_stage
    elif [ -e "$RELEASE_DIR" ] || [ -L "$RELEASE_DIR" ]; then
        fail "Release destination is not a regular directory: $RELEASE_DIR"
        return 1
    else
        mv "$release_stage" "$RELEASE_DIR"
        chmod 0555 "$RELEASE_DIR"
        RELEASE_STAGE=""
    fi
}

copy_legacy_file() {
    legacy_source=$1
    legacy_destination=$2
    legacy_mode=$3
    if [ -L "$legacy_source" ]; then
        fail "Refusing to replace unmanaged symlink $legacy_source."
        return 1
    fi
    if [ -e "$legacy_source" ]; then
        if [ ! -f "$legacy_source" ]; then
            fail "Refusing to replace non-file $legacy_source."
            return 1
        fi
        cp "$legacy_source" "$legacy_destination"
        chmod "$legacy_mode" "$legacy_destination"
        LEGACY_HAS_CONTENT=true
    fi
}

create_legacy_release() {
    legacy_include_systemd=$1
    LEGACY_HAS_CONTENT=false
    legacy_stage="$RELEASES_DIR/.legacy-stage-$$"
    RELEASE_STAGE="$legacy_stage"
    mkdir "$legacy_stage"

    copy_legacy_file "$INSTALL_DIR/$BINARY" "$legacy_stage/$BINARY" 0555 || return 1
    copy_legacy_file "$INSTALL_DIR/$DAEMON_BINARY" "$legacy_stage/$DAEMON_BINARY" 0555 || return 1

    if [ "$legacy_include_systemd" = true ]; then
        legacy_unit_dir="$HOME/.config/systemd/user"
        for legacy_unit in $SYSTEMD_UNITS; do
            legacy_unit_source="$legacy_unit_dir/$legacy_unit"
            if [ -e "$legacy_unit_source" ] || [ -L "$legacy_unit_source" ]; then
                if [ "$LEGACY_HAS_SYSTEMD_DIR" != true ]; then
                    mkdir "$legacy_stage/systemd"
                    LEGACY_HAS_SYSTEMD_DIR=true
                fi
                copy_legacy_file "$legacy_unit_source" "$legacy_stage/systemd/$legacy_unit" 0444 || return 1
            fi
        done
    fi

    if [ "$LEGACY_HAS_CONTENT" != true ]; then
        rmdir "$legacy_stage"
        RELEASE_STAGE=""
        PREVIOUS_TARGET=""
        return 0
    fi

    if [ -d "$legacy_stage/systemd" ]; then
        chmod 0555 "$legacy_stage/systemd"
    fi
    printf '%s\n' "legacy release captured before managed installation" > "$legacy_stage/.legacy"
    chmod 0444 "$legacy_stage/.legacy"
    legacy_base="legacy-$(date +%Y%m%d%H%M%S 2>/dev/null || printf '%s' "$$")-$$"
    legacy_id="$legacy_base"
    legacy_suffix=0
    while [ -e "$RELEASES_DIR/$legacy_id" ] || [ -L "$RELEASES_DIR/$legacy_id" ]; do
        legacy_suffix=$((legacy_suffix + 1))
        legacy_id="$legacy_base-$legacy_suffix"
    done
    legacy_release="$RELEASES_DIR/$legacy_id"
    mv "$legacy_stage" "$legacy_release"
    chmod 0555 "$legacy_release"
    RELEASE_STAGE=""
    PREVIOUS_TARGET=".krusty-releases/$legacy_id"
    atomic_symlink "$PREVIOUS_TARGET" "$CURRENT_LINK"
}

read_previous_release() {
    PREVIOUS_TARGET=""
    if [ -L "$CURRENT_LINK" ]; then
        PREVIOUS_TARGET="$(readlink "$CURRENT_LINK")"
        case "$PREVIOUS_TARGET" in
            .krusty-releases/*) ;;
            *) fail "Refusing unmanaged release pointer $CURRENT_LINK -> $PREVIOUS_TARGET."; return 1 ;;
        esac
        case "/$PREVIOUS_TARGET/" in
            */../*|*/./*) fail "Release pointer contains an unsafe path: $PREVIOUS_TARGET"; return 1 ;;
        esac
        if [ ! -d "$CURRENT_LINK" ]; then
            fail "Current release pointer is dangling: $CURRENT_LINK"
            return 1
        fi
    elif [ -e "$CURRENT_LINK" ]; then
        fail "Refusing non-symlink release pointer $CURRENT_LINK."
        return 1
    fi
}

install_managed_link() {
    managed_target=$1
    managed_path=$2
    allow_legacy_file=$3
    previous_managed_target=${4:-}

    if [ -L "$managed_path" ]; then
        existing_target=$(readlink "$managed_path")
        if [ "$existing_target" != "$managed_target" ] && \
            { [ -z "$previous_managed_target" ] || [ "$existing_target" != "$previous_managed_target" ]; }; then
            fail "Refusing to replace unmanaged symlink $managed_path."
            return 1
        fi
    elif [ -e "$managed_path" ]; then
        if [ "$allow_legacy_file" != true ] || [ ! -f "$managed_path" ]; then
            fail "Refusing to replace unmanaged path $managed_path."
            return 1
        fi
    fi
    atomic_symlink "$managed_target" "$managed_path"
}

managed_systemd_marker_valid() {
    regular_file_with_mode "$SYSTEMD_MARKER" 600 && \
        [ "$(wc -l < "$SYSTEMD_MARKER" | tr -d '[:space:]')" = "1" ] && \
        grep -Fqx "managed by $CURRENT_LINK" "$SYSTEMD_MARKER"
}

previous_managed_unit_target() {
    previous_managed_unit=$1
    [ -n "$PREVIOUS_TARGET" ] || return 1
    managed_systemd_marker_valid || return 1
    printf '%s\n' "$INSTALL_DIR/$PREVIOUS_TARGET/systemd/$previous_managed_unit"
}

snapshot_activation_path() {
    snapshot_key=$1
    snapshot_path=$2
    snapshot_state="$ACTIVATION_BACKUP/$snapshot_key.state"
    if [ -L "$snapshot_path" ]; then
        printf '%s\n' link > "$snapshot_state"
        cp -P "$snapshot_path" "$ACTIVATION_BACKUP/$snapshot_key.link"
    elif [ -f "$snapshot_path" ]; then
        printf '%s\n' file > "$snapshot_state"
        cp -p "$snapshot_path" "$ACTIVATION_BACKUP/$snapshot_key.file"
    elif [ -e "$snapshot_path" ]; then
        fail "Refusing non-file managed path $snapshot_path."
        return 1
    else
        printf '%s\n' absent > "$snapshot_state"
    fi
}

restore_activation_path() {
    restore_key=$1
    restore_path=$2
    restore_state="$ACTIVATION_BACKUP/$restore_key.state"
    restore_kind=$(sed -n '1p' "$restore_state") || return 1
    case "$restore_kind" in
        absent)
            rm -f "$restore_path"
            ;;
        link)
            restore_parent=$(dirname "$restore_path")
            restore_leaf=$(basename "$restore_path")
            restore_tmp="$restore_parent/.$restore_leaf.krusty-restore.$$"
            mkdir -p "$restore_parent"
            rm -f "$restore_tmp"
            ATOMIC_TEMP="$restore_tmp"
            if ! cp -P "$ACTIVATION_BACKUP/$restore_key.link" "$restore_tmp" || [ ! -L "$restore_tmp" ]; then
                cleanup_atomic_temp
                return 1
            fi
            atomic_replace_path "$restore_tmp" "$restore_path" || { cleanup_atomic_temp; return 1; }
            ATOMIC_TEMP=""
            ;;
        file)
            restore_parent=$(dirname "$restore_path")
            restore_leaf=$(basename "$restore_path")
            restore_tmp="$restore_parent/.$restore_leaf.krusty-restore.$$"
            mkdir -p "$restore_parent"
            rm -f "$restore_tmp"
            ATOMIC_TEMP="$restore_tmp"
            if ! cp -p "$ACTIVATION_BACKUP/$restore_key.file" "$restore_tmp"; then
                cleanup_atomic_temp
                return 1
            fi
            atomic_replace_path "$restore_tmp" "$restore_path" || { cleanup_atomic_temp; return 1; }
            ATOMIC_TEMP=""
            ;;
        *)
            fail "Invalid activation snapshot for $restore_path."
            return 1
            ;;
    esac
}

prepare_activation_snapshot() {
    ACTIVATION_BACKUP="$INSTALL_DIR/.krusty-activation-backup.$$"
    if [ -e "$ACTIVATION_BACKUP" ] || [ -L "$ACTIVATION_BACKUP" ]; then
        fail "Unexpected activation backup already exists: $ACTIVATION_BACKUP"
        return 1
    fi
    mkdir "$ACTIVATION_BACKUP"
    chmod 0700 "$ACTIVATION_BACKUP"
    SYSTEMD_USER_DIR_WAS_PRESENT=false
    [ -d "$SYSTEMD_USER_DIR" ] && SYSTEMD_USER_DIR_WAS_PRESENT=true
    snapshot_activation_path current "$CURRENT_LINK" || return 1
    snapshot_activation_path krusty "$INSTALL_DIR/$BINARY" || return 1
    snapshot_activation_path mako "$INSTALL_DIR/$DAEMON_BINARY" || return 1
    if [ "$MANAGE_SYSTEMD" = true ]; then
        snapshot_activation_path marker "$SYSTEMD_MARKER" || return 1
        for snapshot_unit in $SYSTEMD_UNITS; do
            case "$snapshot_unit" in
                krusty-mako.socket) snapshot_key=mako-socket ;;
                krusty-mako.service) snapshot_key=mako-service ;;
                krusty-serve.service) snapshot_key=serve-service ;;
            esac
            snapshot_activation_path "$snapshot_key" "$SYSTEMD_USER_DIR/$snapshot_unit" || return 1
        done
    fi
}

restore_activation_snapshot() {
    restore_failed=false
    restore_activation_path current "$CURRENT_LINK" || restore_failed=true
    restore_activation_path krusty "$INSTALL_DIR/$BINARY" || restore_failed=true
    restore_activation_path mako "$INSTALL_DIR/$DAEMON_BINARY" || restore_failed=true
    if [ "$MANAGE_SYSTEMD" = true ]; then
        restore_activation_path marker "$SYSTEMD_MARKER" || restore_failed=true
        for restore_unit in $SYSTEMD_UNITS; do
            case "$restore_unit" in
                krusty-mako.socket) restore_key=mako-socket ;;
                krusty-mako.service) restore_key=mako-service ;;
                krusty-serve.service) restore_key=serve-service ;;
            esac
            restore_activation_path "$restore_key" "$SYSTEMD_USER_DIR/$restore_unit" || restore_failed=true
        done
    fi
    [ "$restore_failed" = false ]
}

publish_systemd_marker() {
    if [ -L "$SYSTEMD_MARKER" ] || { [ -e "$SYSTEMD_MARKER" ] && [ ! -f "$SYSTEMD_MARKER" ]; }; then
        fail "Refusing non-regular systemd marker $SYSTEMD_MARKER."
        return 1
    fi
    marker_parent=$(dirname "$SYSTEMD_MARKER")
    marker_leaf=$(basename "$SYSTEMD_MARKER")
    marker_tmp="$marker_parent/.$marker_leaf.krusty-new.$$"
    rm -f "$marker_tmp"
    ATOMIC_TEMP="$marker_tmp"
    if ! (umask 077; printf '%s\n' "managed by $CURRENT_LINK" > "$marker_tmp"); then
        cleanup_atomic_temp
        return 1
    fi
    chmod 0600 "$marker_tmp" || { cleanup_atomic_temp; return 1; }
    activation_checkpoint publish-marker || { cleanup_atomic_temp; return 1; }
    atomic_replace_path "$marker_tmp" "$SYSTEMD_MARKER" || { cleanup_atomic_temp; return 1; }
    ATOMIC_TEMP=""
    regular_file_with_mode "$SYSTEMD_MARKER" 600
}

activation_checkpoint() {
    checkpoint_name=$1
    if [ "$SELF_TEST_SIGNAL_POINT" = "$checkpoint_name" ]; then
        activation_signal "fixture at $checkpoint_name"
        return 1
    fi
    [ "$SELF_TEST_FAIL_POINT" != "$checkpoint_name" ]
}

systemctl_available() {
    command -v systemctl >/dev/null 2>&1
}

run_systemctl() {
    systemctl "$@"
}

capture_active_services() {
    ACTIVE_UNITS=""
    if [ "$MANAGE_SYSTEMD" != true ] || ! systemctl_available; then
        return 0
    fi
    for active_unit in $SYSTEMD_UNITS; do
        if run_systemctl --user is-active --quiet "$active_unit" >/dev/null 2>&1; then
            ACTIVE_UNITS="$ACTIVE_UNITS $active_unit"
        fi
    done
}

unit_was_active() {
    case " $ACTIVE_UNITS " in
        *" $1 "*) return 0 ;;
        *) return 1 ;;
    esac
}

health_pause() {
    sleep 1
}

restart_previously_active() {
    [ -n "$ACTIVE_UNITS" ] || return 0
    # ACTIVE_UNITS is assembled only from the constant SYSTEMD_UNITS list.
    # shellcheck disable=SC2086
    run_systemctl --user restart $ACTIVE_UNITS
}

verify_previously_active() {
    [ -n "$ACTIVE_UNITS" ] || return 0
    health_attempt=1
    stable_health_passes=0
    while [ "$health_attempt" -le "$ACTIVATION_HEALTH_ATTEMPTS" ]; do
        health_pause
        all_units_active=true
        for health_unit in $ACTIVE_UNITS; do
            if ! run_systemctl --user is-active --quiet "$health_unit" >/dev/null 2>&1; then
                all_units_active=false
                break
            fi
        done
        if [ "$all_units_active" = true ]; then
            stable_health_passes=$((stable_health_passes + 1))
            if [ "$stable_health_passes" -ge "$ACTIVATION_STABLE_PASSES" ]; then
                return 0
            fi
        else
            stable_health_passes=0
        fi
        health_attempt=$((health_attempt + 1))
    done
    return 1
}

restart_and_verify_previously_active() {
    restart_previously_active || return 1
    verify_previously_active
}

stop_newly_active_candidate_units() {
    [ "$SYSTEMD_TRANSITION_STARTED" = true ] || return 0
    candidate_new_units=""
    for candidate_unit in $SYSTEMD_UNITS; do
        if ! unit_was_active "$candidate_unit" && \
            run_systemctl --user is-active --quiet "$candidate_unit" >/dev/null 2>&1; then
            candidate_new_units="$candidate_new_units $candidate_unit"
        fi
    done
    [ -n "$candidate_new_units" ] || return 0
    # candidate_new_units is assembled only from SYSTEMD_UNITS.
    # shellcheck disable=SC2086
    run_systemctl --user stop $candidate_new_units
}

rollback_release() {
    rollback_reason=$1
    echo "Activation failed ($rollback_reason); rolling back the release pointer." >&2
    ACTIVATION_ROLLING_BACK=true
    ACTIVATION_IN_PROGRESS=false
    trap '' HUP INT TERM
    cleanup_atomic_temp
    discard_release_stage
    rollback_failed=false
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ] && systemctl_available; then
        if ! stop_newly_active_candidate_units; then
            echo "Warning: candidate-only services could not all be stopped." >&2
            rollback_failed=true
        fi
    fi
    if ! restore_activation_snapshot; then
        echo "Warning: one or more managed paths could not be restored." >&2
        rollback_failed=true
    fi
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_USER_DIR_WAS_PRESENT" != true ]; then
        rmdir "$SYSTEMD_USER_DIR" 2>/dev/null || true
    fi
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ] && systemctl_available; then
        if ! run_systemctl --user daemon-reload; then
            echo "Warning: systemd could not reload the restored unit set." >&2
            rollback_failed=true
        fi
        if ! restart_and_verify_previously_active; then
            echo "Warning: one or more previously active services could not be restored healthy." >&2
            rollback_failed=true
        fi
    fi
    cleanup_activation_backup
    ACTIVATION_ROLLING_BACK=false
    SYSTEMD_TRANSITION_STARTED=false
    arm_signal_traps
    [ "$rollback_failed" = false ] || echo "Warning: rollback completed with errors." >&2
    return 1
}

fail_activation() {
    fail_activation_reason=$1
    if [ "$ACTIVATION_IN_PROGRESS" = true ]; then
        rollback_release "$fail_activation_reason" || true
    fi
    return 1
}

activate_unix_release() {
    CURRENT_LINK="$INSTALL_DIR/.krusty-current"
    SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
    SYSTEMD_MARKER="$INSTALL_DIR/.krusty-systemd-managed"
    MIGRATING_LEGACY=false
    LEGACY_HAS_SYSTEMD_DIR=false
    SYSTEMD_TRANSITION_STARTED=false

    read_previous_release || return 1

    if [ "$OS" = "Linux" ] && [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ] && \
        { [ -L "$SYSTEMD_MARKER" ] || { [ -e "$SYSTEMD_MARKER" ] && [ ! -f "$SYSTEMD_MARKER" ]; }; }; then
        fail "Refusing non-regular systemd marker $SYSTEMD_MARKER."
        return 1
    fi

    EXISTING_SUPERVISED_SET=false
    if [ -f "$CURRENT_LINK/$DAEMON_BINARY" ] || [ -e "$INSTALL_DIR/$DAEMON_BINARY" ] || \
        [ -L "$INSTALL_DIR/$DAEMON_BINARY" ]; then
        EXISTING_SUPERVISED_SET=true
    fi
    if [ "$OS" = "Linux" ] && [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ]; then
        if [ -f "$SYSTEMD_MARKER" ]; then
            EXISTING_SUPERVISED_SET=true
        fi
        for existing_unit in $SYSTEMD_UNITS; do
            if [ -e "$SYSTEMD_USER_DIR/$existing_unit" ] || [ -L "$SYSTEMD_USER_DIR/$existing_unit" ]; then
                EXISTING_SUPERVISED_SET=true
            fi
        done
    fi
    if [ "$EXISTING_SUPERVISED_SET" = true ] && [ ! -f "$RELEASE_DIR/$DAEMON_BINARY" ]; then
        fail "Refusing to replace a supervised Hive release with a krusty-only archive."
        return 1
    fi

    MANAGE_SYSTEMD=false
    if [ "$OS" = "Linux" ] && [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ]; then
        if [ -d "$RELEASE_DIR/systemd" ] || [ -f "$SYSTEMD_MARKER" ] || \
            { [ -n "$PREVIOUS_TARGET" ] && [ -d "$CURRENT_LINK/systemd" ]; }; then
            MANAGE_SYSTEMD=true
        fi
    elif [ "$OS" = "Linux" ] && [ -d "$RELEASE_DIR/systemd" ]; then
        echo "Skipping systemd units: custom INSTALL_DIR is not in the shipped unit PATH."
        echo "Install and override the units explicitly if supervision is required."
    fi

    capture_active_services
    if ! prepare_activation_snapshot; then
        cleanup_activation_backup
        return 1
    fi
    ACTIVATION_IN_PROGRESS=true

    if [ -z "$PREVIOUS_TARGET" ]; then
        MIGRATING_LEGACY=true
        create_legacy_release "$MANAGE_SYSTEMD" || { fail_activation "legacy release capture failed"; return 1; }
    fi

    install_managed_link ".krusty-current/$BINARY" "$INSTALL_DIR/$BINARY" "$MIGRATING_LEGACY" || \
        { fail_activation "$BINARY link publication failed"; return 1; }
    activation_checkpoint after-krusty-link || { fail_activation "fixture after $BINARY link"; return 1; }
    if [ -f "$RELEASE_DIR/$DAEMON_BINARY" ] || [ -f "$CURRENT_LINK/$DAEMON_BINARY" ] || \
        [ -e "$INSTALL_DIR/$DAEMON_BINARY" ] || [ -L "$INSTALL_DIR/$DAEMON_BINARY" ]; then
        install_managed_link ".krusty-current/$DAEMON_BINARY" "$INSTALL_DIR/$DAEMON_BINARY" "$MIGRATING_LEGACY" || \
            { fail_activation "$DAEMON_BINARY link publication failed"; return 1; }
        activation_checkpoint after-mako-link || { fail_activation "fixture after $DAEMON_BINARY link"; return 1; }
    fi

    if [ "$MANAGE_SYSTEMD" = true ]; then
        mkdir -p "$SYSTEMD_USER_DIR" || { fail_activation "systemd user directory creation failed"; return 1; }
        for managed_unit in $SYSTEMD_UNITS; do
            previous_unit_target=""
            previous_unit_target=$(previous_managed_unit_target "$managed_unit") || previous_unit_target=""
            install_managed_link "$CURRENT_LINK/systemd/$managed_unit" "$SYSTEMD_USER_DIR/$managed_unit" \
                "$MIGRATING_LEGACY" "$previous_unit_target" || \
                { fail_activation "$managed_unit link publication failed"; return 1; }
            activation_checkpoint "after-$managed_unit-link" || \
                { fail_activation "fixture after $managed_unit link"; return 1; }
        done
        activation_checkpoint before-marker || { fail_activation "fixture before marker publication"; return 1; }
        publish_systemd_marker || { fail_activation "systemd marker publication failed"; return 1; }
        activation_checkpoint after-marker || { fail_activation "fixture after marker publication"; return 1; }
    fi

    if ! atomic_symlink ".krusty-releases/$RELEASE_ID" "$CURRENT_LINK"; then
        fail_activation "could not activate release $RELEASE_ID"
        return 1
    fi
    activation_checkpoint after-pointer || { fail_activation "fixture after release pointer"; return 1; }

    if [ "$MANAGE_SYSTEMD" = true ]; then
        if systemctl_available; then
            SYSTEMD_TRANSITION_STARTED=true
            if ! run_systemctl --user daemon-reload; then
                fail_activation "systemd daemon-reload failed"
                return 1
            fi
            if ! restart_and_verify_previously_active; then
                fail_activation "a previously active service did not settle healthy"
                return 1
            fi
        else
            echo "systemctl is unavailable; units were installed but not reloaded."
        fi
    fi

    ACTIVATION_IN_PROGRESS=false
    cleanup_activation_backup
    SYSTEMD_TRANSITION_STARTED=false
    INSTALLED_SYSTEMD_UNITS="$MANAGE_SYSTEMD"
}

install_windows_direct() {
    windows_payload=$1
    windows_source="$windows_payload/$BINARY"
    if [ ! -f "$windows_source" ] && [ -f "$windows_payload/$BINARY.exe" ]; then
        windows_source="$windows_payload/$BINARY.exe"
    fi
    if [ ! -f "$windows_source" ] || [ -L "$windows_source" ]; then
        fail "The verified archive does not contain a regular $BINARY.exe binary."
        return 1
    fi

    echo "Installing directly to $INSTALL_DIR..."
    acquire_install_lock || return 1

    windows_destination="$INSTALL_DIR/$BINARY.exe"
    if [ -e "$windows_destination" ] || [ -L "$windows_destination" ]; then
        if [ ! -f "$windows_destination" ] || [ -L "$windows_destination" ]; then
            fail "Refusing to replace non-regular Windows destination $windows_destination."
            return 1
        fi
    fi

    windows_stage="$INSTALL_DIR/.$BINARY.exe.krusty-new.$$"
    if [ -e "$windows_stage" ] || [ -L "$windows_stage" ]; then
        fail "Unexpected Windows staging path already exists: $windows_stage"
        return 1
    fi

    ATOMIC_TEMP="$windows_stage"
    if ! cp "$windows_source" "$windows_stage" || ! chmod 0755 "$windows_stage"; then
        cleanup_atomic_temp
        fail "Could not stage $BINARY.exe in $INSTALL_DIR."
        return 1
    fi
    if [ "$SELF_TEST_FAIL_POINT" = windows-before-publish ]; then
        cleanup_atomic_temp
        fail "Fixture stopped Windows installation before publication."
        return 1
    fi

    # Recheck immediately before publication. The install lock excludes other
    # cooperating installers, and this also fails closed if the path changed.
    if [ -e "$windows_destination" ] || [ -L "$windows_destination" ]; then
        if [ ! -f "$windows_destination" ] || [ -L "$windows_destination" ]; then
            cleanup_atomic_temp
            fail "Refusing to replace non-regular Windows destination $windows_destination."
            return 1
        fi
    fi
    if ! atomic_replace_path "$windows_stage" "$windows_destination"; then
        cleanup_atomic_temp
        fail "Could not atomically publish $windows_destination."
        return 1
    fi
    ATOMIC_TEMP=""
}

run_self_test() (
    set -e
    self_root="$(mktemp -d)"
    self_cleanup() {
        chmod -R u+w "$self_root" 2>/dev/null || true
        rm -rf "$self_root"
    }
    trap self_cleanup 0 HUP INT TERM
    HOME="$self_root/home"
    export HOME
    DEFAULT_INSTALL_DIR="$HOME/.local/bin"
    INSTALL_DIR="$DEFAULT_INSTALL_DIR"
    OS="Linux"
    SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
    RELEASES_DIR="$INSTALL_DIR/.krusty-releases"
    mkdir -p "$RELEASES_DIR" "$SYSTEMD_USER_DIR"
    printf '#!/bin/sh\nprintf "%%s\\n" "direct"\n' > "$INSTALL_DIR/krusty"
    chmod 0755 "$INSTALL_DIR/krusty"
    printf '%s\n' '# legacy serve only' > "$SYSTEMD_USER_DIR/krusty-serve.service"
    chmod 0644 "$SYSTEMD_USER_DIR/krusty-serve.service"

    systemctl_available() { return 0; }
    health_pause() { :; }
    SELF_ACTIVE_UNITS=" krusty-serve.service"
    SELF_LOG=""
    SELF_AFTER_RESTART=false
    SELF_ADD_DEPENDENCY=false
    SELF_FAIL_RELOAD_ONCE=false
    SELF_FAIL_RESTART_ONCE=false
    SELF_FAIL_HEALTH_COUNT=0
    SELF_HEALTH_CHECKS=0

    self_unit_active() {
        case " $SELF_ACTIVE_UNITS " in
            *" $1 "*) return 0 ;;
            *) return 1 ;;
        esac
    }
    self_add_active() {
        self_unit_active "$1" || SELF_ACTIVE_UNITS="$SELF_ACTIVE_UNITS $1"
    }
    self_remove_active() {
        self_remaining=""
        for self_active in $SELF_ACTIVE_UNITS; do
            [ "$self_active" = "$1" ] || self_remaining="$self_remaining $self_active"
        done
        SELF_ACTIVE_UNITS="$self_remaining"
    }
    run_systemctl() {
        [ "$1" = "--user" ] || return 90
        shift
        self_command=$1
        shift
        case "$self_command" in
            is-active)
                [ "$1" = "--quiet" ] || return 91
                shift
                if [ "$SELF_AFTER_RESTART" = true ] && [ "$1" = "krusty-serve.service" ]; then
                    SELF_HEALTH_CHECKS=$((SELF_HEALTH_CHECKS + 1))
                    if [ "$SELF_FAIL_HEALTH_COUNT" -gt 0 ]; then
                        SELF_FAIL_HEALTH_COUNT=$((SELF_FAIL_HEALTH_COUNT - 1))
                        return 1
                    fi
                fi
                self_unit_active "$1"
                ;;
            daemon-reload)
                SELF_LOG="$SELF_LOG|daemon-reload"
                if [ "$SELF_FAIL_RELOAD_ONCE" = true ]; then
                    SELF_FAIL_RELOAD_ONCE=false
                    return 1
                fi
                ;;
            restart)
                [ "$#" -eq 1 ] && [ "$1" = "krusty-serve.service" ] || return 92
                SELF_LOG="$SELF_LOG|restart:$1"
                self_add_active "$1"
                SELF_AFTER_RESTART=true
                [ "$SELF_ADD_DEPENDENCY" = true ] && self_add_active krusty-mako.socket
                if [ "$SELF_FAIL_RESTART_ONCE" = true ]; then
                    SELF_FAIL_RESTART_ONCE=false
                    return 1
                fi
                ;;
            stop)
                SELF_LOG="$SELF_LOG|stop:$*"
                for self_stopped in "$@"; do self_remove_active "$self_stopped"; done
                ;;
            *) return 93 ;;
        esac
    }

    write_self_test_payload() {
        self_payload=$1
        self_value=$2
        self_kind=$3
        mkdir -p "$self_payload"
        printf '#!/bin/sh\nprintf "%%s\\n" "%s"\n' "$self_value" > "$self_payload/krusty"
        chmod 0755 "$self_payload/krusty"
        if [ "$self_kind" = complete ]; then
            mkdir "$self_payload/systemd"
            printf '#!/bin/sh\nprintf "%%s\\n" "mako-%s"\n' "$self_value" > "$self_payload/krusty-mako"
            chmod 0755 "$self_payload/krusty-mako"
            for self_unit in $SYSTEMD_UNITS; do
                printf '# fixture %s %s\n' "$self_unit" "$self_value" > "$self_payload/systemd/$self_unit"
            done
        fi
    }
    stage_self_test_release() {
        VERSION=$1
        self_value=$2
        self_kind=$3
        ARCHIVE_SHA256=$4
        PLATFORM=self-test
        self_payload="$self_root/payload-$self_value"
        write_self_test_payload "$self_payload" "$self_value" "$self_kind"
        stage_unix_release "$self_payload"
    }
    assert_no_activation_residue() {
        [ -z "$ACTIVATION_BACKUP" ]
        [ -z "$ATOMIC_TEMP" ]
        for self_residue in \
            "$INSTALL_DIR"/.krusty-activation-backup.* \
            "$INSTALL_DIR"/.*.krusty-new.* \
            "$INSTALL_DIR"/.*.krusty-restore.* \
            "$SYSTEMD_USER_DIR"/.*.krusty-new.* \
            "$SYSTEMD_USER_DIR"/.*.krusty-restore.*; do
            [ ! -e "$self_residue" ] && [ ! -L "$self_residue" ] || return 1
        done
    }
    assert_direct_baseline() {
        [ ! -e "$INSTALL_DIR/.krusty-current" ] && [ ! -L "$INSTALL_DIR/.krusty-current" ]
        [ -f "$INSTALL_DIR/krusty" ] && [ ! -L "$INSTALL_DIR/krusty" ]
        [ "$("$INSTALL_DIR/krusty")" = direct ]
        [ ! -e "$INSTALL_DIR/krusty-mako" ] && [ ! -L "$INSTALL_DIR/krusty-mako" ]
        [ -f "$SYSTEMD_USER_DIR/krusty-serve.service" ] && [ ! -L "$SYSTEMD_USER_DIR/krusty-serve.service" ]
        grep -Fqx '# legacy serve only' "$SYSTEMD_USER_DIR/krusty-serve.service"
        [ ! -e "$SYSTEMD_USER_DIR/krusty-mako.socket" ] && [ ! -L "$SYSTEMD_USER_DIR/krusty-mako.socket" ]
        [ ! -e "$SYSTEMD_USER_DIR/krusty-mako.service" ] && [ ! -L "$SYSTEMD_USER_DIR/krusty-mako.service" ]
        [ ! -e "$INSTALL_DIR/.krusty-systemd-managed" ] && [ ! -L "$INSTALL_DIR/.krusty-systemd-managed" ]
        assert_no_activation_residue
    }
    reset_self_systemd() {
        SELF_ACTIVE_UNITS=" krusty-serve.service"
        SELF_LOG=""
        SELF_AFTER_RESTART=false
        SELF_ADD_DEPENDENCY=false
        SELF_FAIL_RELOAD_ONCE=false
        SELF_FAIL_RESTART_ONCE=false
        SELF_FAIL_HEALTH_COUNT=0
        SELF_HEALTH_CHECKS=0
    }
    release_self_install_lock() {
        [ "$LOCK_HELD" = true ]
        [ "$INSTALL_LOCK" = "$INSTALL_DIR/.krusty-install.lock" ]
        rmdir "$INSTALL_LOCK"
        LOCK_HELD=false
        INSTALL_LOCK=""
    }
    assert_no_windows_stage() {
        [ -z "$ATOMIC_TEMP" ]
        for self_windows_stage in "$INSTALL_DIR"/.$BINARY.exe.krusty-new.*; do
            [ ! -e "$self_windows_stage" ] && [ ! -L "$self_windows_stage" ] || return 1
        done
    }
    assert_unmanaged_symlink_rollback() {
        self_link_target=$1
        self_link_label=$2
        self_link_path="$INSTALL_DIR/krusty-mako"
        self_expected="$self_root/$self_link_label.expected"
        self_actual="$self_root/$self_link_label.actual"
        ln -s "$self_link_target" "$self_link_path"
        readlink "$self_link_path" > "$self_expected"
        reset_self_systemd
        if activate_unix_release; then
            fail "Self-test accepted unmanaged $self_link_label symlink."
            exit 1
        fi
        [ -L "$self_link_path" ]
        readlink "$self_link_path" > "$self_actual"
        cmp -s "$self_expected" "$self_actual"
        [ -z "$SELF_LOG" ]
        rm -f "$self_link_path" "$self_expected" "$self_actual"
        assert_direct_baseline
    }

    stage_self_test_release v1 one complete \
        1111111111111111111111111111111111111111111111111111111111111111
    v1_release_id=$RELEASE_ID
    v1_release_dir=$RELEASE_DIR
    stage_unix_release "$self_root/payload-one"
    [ -z "$RELEASE_STAGE" ]
    chmod 0755 "$v1_release_dir/krusty"
    if stage_unix_release "$self_root/payload-one"; then
        fail "Self-test accepted a release with a changed primary mode."
        exit 1
    fi
    chmod 0555 "$v1_release_dir/krusty"
    [ -z "$RELEASE_STAGE" ]

    multiline_target=$(printf 'line-one\nline-two')
    trailing_newline_target=$(printf 'line-one\nline-two\nsentinel')
    trailing_newline_target=${trailing_newline_target%sentinel}
    assert_unmanaged_symlink_rollback "$multiline_target" multiline
    assert_unmanaged_symlink_rollback "$trailing_newline_target" trailing-newline

    marker_target="$self_root/marker-target"
    printf '%s\n' marker > "$marker_target"
    ln -s "$marker_target" "$INSTALL_DIR/.krusty-systemd-managed"
    if activate_unix_release; then fail "Self-test accepted a symlink marker."; exit 1; fi
    [ -L "$INSTALL_DIR/.krusty-systemd-managed" ]
    rm -f "$INSTALL_DIR/.krusty-systemd-managed"
    mkdir "$INSTALL_DIR/.krusty-systemd-managed"
    if activate_unix_release; then fail "Self-test accepted a directory marker."; exit 1; fi
    rmdir "$INSTALL_DIR/.krusty-systemd-managed"
    assert_direct_baseline

    for failure_point in \
        after-krusty-link \
        after-mako-link \
        after-krusty-mako.socket-link \
        after-krusty-mako.service-link \
        after-krusty-serve.service-link \
        publish-marker \
        after-pointer; do
        SELF_TEST_FAIL_POINT=$failure_point
        if activate_unix_release; then
            fail "Self-test expected activation fault at $failure_point."
            exit 1
        fi
        SELF_TEST_FAIL_POINT=""
        [ -z "$SELF_LOG" ]
        assert_direct_baseline
    done

    SELF_TEST_CAPTURE_SIGNAL=true
    SELF_TEST_SIGNAL_POINT=after-marker
    if activate_unix_release; then fail "Self-test expected signal rollback."; exit 1; fi
    SELF_TEST_CAPTURE_SIGNAL=false
    SELF_TEST_SIGNAL_POINT=""
    assert_direct_baseline

    reset_self_systemd
    activate_unix_release
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]
    [ "$("$INSTALL_DIR/krusty")" = one ]
    regular_file_with_mode "$INSTALL_DIR/.krusty-systemd-managed" 600
    v1_marker_contents=$(sed -n '1p' "$INSTALL_DIR/.krusty-systemd-managed")
    [ "$SELF_HEALTH_CHECKS" -ge 2 ]
    assert_no_activation_residue
    serve_only_legacy=false
    for legacy_candidate in "$RELEASES_DIR"/legacy-*; do
        if [ -f "$legacy_candidate/systemd/krusty-serve.service" ] && \
            [ ! -e "$legacy_candidate/systemd/krusty-mako.socket" ] && \
            [ ! -e "$legacy_candidate/systemd/krusty-mako.service" ]; then
            serve_only_legacy=true
        fi
    done
    [ "$serve_only_legacy" = true ]

    # Older supervised installs linked units directly into the selected
    # immutable release. A valid ownership marker plus an exact match to the
    # current release is sufficient to migrate those links to .krusty-current.
    for legacy_managed_unit in $SYSTEMD_UNITS; do
        rm -f "$SYSTEMD_USER_DIR/$legacy_managed_unit"
        ln -s "$v1_release_dir/systemd/$legacy_managed_unit" \
            "$SYSTEMD_USER_DIR/$legacy_managed_unit"
    done
    reset_self_systemd
    activate_unix_release
    for migrated_unit in $SYSTEMD_UNITS; do
        [ "$(readlink "$SYSTEMD_USER_DIR/$migrated_unit")" = \
            "$INSTALL_DIR/.krusty-current/systemd/$migrated_unit" ]
    done
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]
    [ "$SELF_HEALTH_CHECKS" -ge 2 ]
    assert_no_activation_residue

    # A service can fail its first health sample while systemd applies its
    # configured restart policy. Activation should accept eventual stability.
    reset_self_systemd
    ACTIVE_UNITS=" krusty-serve.service"
    SELF_AFTER_RESTART=true
    SELF_FAIL_HEALTH_COUNT=1
    verify_previously_active
    [ "$SELF_HEALTH_CHECKS" -ge 3 ]

    # The marker does not authorize unrelated absolute unit links.
    rejected_unit="$SYSTEMD_USER_DIR/krusty-mako.socket"
    rejected_target="$self_root/unmanaged-krusty-mako.socket"
    printf '%s\n' unmanaged > "$rejected_target"
    rm -f "$rejected_unit"
    ln -s "$rejected_target" "$rejected_unit"
    reset_self_systemd
    if activate_unix_release; then
        fail "Self-test accepted an unrelated absolute systemd unit symlink."
        exit 1
    fi
    [ "$(readlink "$rejected_unit")" = "$rejected_target" ]
    [ -z "$SELF_LOG" ]
    rm -f "$rejected_unit"
    ln -s "$INSTALL_DIR/.krusty-current/systemd/krusty-mako.socket" "$rejected_unit"
    assert_no_activation_residue

    stage_self_test_release v0.7.3-downgrade downgrade krusty-only \
        2222222222222222222222222222222222222222222222222222222222222222
    if activate_unix_release; then fail "Self-test accepted a supervised downgrade."; exit 1; fi
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]
    assert_no_activation_residue

    stage_self_test_release v2 two complete \
        3333333333333333333333333333333333333333333333333333333333333333
    v2_release_dir=$RELEASE_DIR
    reset_self_systemd
    SELF_FAIL_RELOAD_ONCE=true
    if activate_unix_release; then fail "Self-test expected daemon-reload rollback."; exit 1; fi
    case "$SELF_LOG" in
        '|daemon-reload|daemon-reload|restart:krusty-serve.service') ;;
        *) fail "Daemon-reload rollback ordering was incorrect: $SELF_LOG"; exit 1 ;;
    esac
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]

    reset_self_systemd
    SELF_ADD_DEPENDENCY=true
    SELF_FAIL_RESTART_ONCE=true
    if activate_unix_release; then fail "Self-test expected restart rollback."; exit 1; fi
    case "$SELF_LOG" in
        *'|restart:krusty-serve.service|stop:krusty-mako.socket|daemon-reload|restart:krusty-serve.service'*) ;;
        *) fail "Candidate dependency was not stopped before rollback reload: $SELF_LOG"; exit 1 ;;
    esac
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]
    regular_file_with_mode "$INSTALL_DIR/.krusty-systemd-managed" 600
    [ "$(sed -n '1p' "$INSTALL_DIR/.krusty-systemd-managed")" = "$v1_marker_contents" ]
    [ -d "$v2_release_dir" ]
    assert_no_activation_residue

    reset_self_systemd
    SELF_ADD_DEPENDENCY=true
    SELF_FAIL_HEALTH_COUNT=$ACTIVATION_HEALTH_ATTEMPTS
    if activate_unix_release; then fail "Self-test expected unhealthy-service rollback."; exit 1; fi
    case "$SELF_LOG" in
        *'|restart:krusty-serve.service|stop:krusty-mako.socket|daemon-reload|restart:krusty-serve.service'*) ;;
        *) fail "Health rollback ordering was incorrect: $SELF_LOG"; exit 1 ;;
    esac
    [ "$SELF_HEALTH_CHECKS" -ge 3 ]
    [ "$(readlink "$INSTALL_DIR/.krusty-current")" = ".krusty-releases/$v1_release_id" ]
    regular_file_with_mode "$INSTALL_DIR/.krusty-systemd-managed" 600
    [ "$(sed -n '1p' "$INSTALL_DIR/.krusty-systemd-managed")" = "$v1_marker_contents" ]
    assert_no_activation_residue

    printf './krusty\nsystemd/krusty-serve.service\n' | safe_member_paths
    if printf '../escape\n' | safe_member_paths; then fail "Self-test accepted traversal."; exit 1; fi
    printf '%s\n' '-rwxr-xr-x fixture' 'drwxr-xr-x systemd' | safe_tar_member_types
    if printf '%s\n' 'lrwxr-xr-x escape' | safe_tar_member_types; then fail "Self-test accepted tar symlink."; exit 1; fi
    printf '%s\n' 'Archive: fixture' '-rwxr-xr-x fixture' | safe_zip_member_types 1
    if printf '%s\n' '?rwxr-xr-x fixture' | safe_zip_member_types 1; then fail "Self-test accepted unknown ZIP type."; exit 1; fi
    EXT=tar.gz
    tar czf "$self_root/good.tar.gz" -C "$self_root/payload-one" .
    preflight_archive "$self_root/good.tar.gz"
    ln -s krusty "$self_root/payload-one/escape"
    tar czf "$self_root/symlink.tar.gz" -C "$self_root/payload-one" .
    if preflight_archive "$self_root/symlink.tar.gz"; then fail "Self-test accepted tar symlink entry."; exit 1; fi
    if command -v zip >/dev/null 2>&1 && command -v unzip >/dev/null 2>&1; then
        mkdir "$self_root/zip-payload"
        printf '%s\n' zip > "$self_root/zip-payload/krusty.exe"
        (cd "$self_root/zip-payload" && zip -q "$self_root/good.zip" krusty.exe)
        EXT=zip
        preflight_archive "$self_root/good.zip"
        ln -s krusty.exe "$self_root/zip-payload/escape"
        (cd "$self_root/zip-payload" && zip -qy "$self_root/symlink.zip" escape)
        if preflight_archive "$self_root/symlink.zip"; then fail "Self-test accepted ZIP symlink entry."; exit 1; fi
    fi

    windows_payload="$self_root/windows-direct-payload"
    windows_install_dir="$self_root/windows-bin"
    mkdir -p "$windows_payload" "$windows_install_dir"
    printf '%s\n' windows-new > "$windows_payload/krusty.exe"
    cp "$windows_payload/krusty.exe" "$self_root/windows-payload.expected"
    INSTALL_DIR="$windows_install_dir"

    mkdir "$INSTALL_DIR/krusty.exe"
    printf '%s\n' directory-sentinel > "$INSTALL_DIR/krusty.exe/sentinel"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted a directory Windows destination."
        exit 1
    fi
    [ "$LOCK_HELD" = true ]
    grep -Fqx directory-sentinel "$INSTALL_DIR/krusty.exe/sentinel"
    [ ! -e "$INSTALL_DIR/krusty.exe/krusty.exe" ] && [ ! -L "$INSTALL_DIR/krusty.exe/krusty.exe" ]
    cmp -s "$windows_payload/krusty.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock
    remove_writable_tree "$INSTALL_DIR/krusty.exe"

    mkdir "$self_root/windows-link-target"
    printf '%s\n' symlink-sentinel > "$self_root/windows-link-target/sentinel"
    ln -s "$self_root/windows-link-target" "$INSTALL_DIR/krusty.exe"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted a symlink Windows destination."
        exit 1
    fi
    [ "$LOCK_HELD" = true ]
    [ -L "$INSTALL_DIR/krusty.exe" ]
    [ "$(readlink "$INSTALL_DIR/krusty.exe")" = "$self_root/windows-link-target" ]
    grep -Fqx symlink-sentinel "$self_root/windows-link-target/sentinel"
    [ ! -e "$self_root/windows-link-target/krusty.exe" ] && [ ! -L "$self_root/windows-link-target/krusty.exe" ]
    cmp -s "$windows_payload/krusty.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock
    rm -f "$INSTALL_DIR/krusty.exe"

    printf '%s\n' windows-old > "$INSTALL_DIR/krusty.exe"
    cp "$INSTALL_DIR/krusty.exe" "$self_root/windows-destination.expected"
    SELF_TEST_FAIL_POINT=windows-before-publish
    if install_windows_direct "$windows_payload"; then
        fail "Self-test expected Windows pre-publication rollback."
        exit 1
    fi
    SELF_TEST_FAIL_POINT=""
    [ "$LOCK_HELD" = true ]
    cmp -s "$INSTALL_DIR/krusty.exe" "$self_root/windows-destination.expected"
    cmp -s "$windows_payload/krusty.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock

    install_windows_direct "$windows_payload"
    [ "$LOCK_HELD" = true ]
    regular_file_with_mode "$INSTALL_DIR/krusty.exe" 755
    cmp -s "$INSTALL_DIR/krusty.exe" "$self_root/windows-payload.expected"
    cmp -s "$windows_payload/krusty.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock

    readonly_tree="$self_root/readonly-cleanup"
    mkdir -p "$readonly_tree/child"
    printf '%s\n' readonly > "$readonly_tree/child/file"
    chmod 0444 "$readonly_tree/child/file"
    chmod 0555 "$readonly_tree/child" "$readonly_tree"
    remove_writable_tree "$readonly_tree"
    [ ! -e "$readonly_tree" ]

    HOME="$self_root/compat-home"
    export HOME
    DEFAULT_INSTALL_DIR="$HOME/.local/bin"
    INSTALL_DIR="$DEFAULT_INSTALL_DIR"
    RELEASES_DIR="$INSTALL_DIR/.krusty-releases"
    mkdir -p "$RELEASES_DIR"
    reset_self_systemd
    stage_self_test_release v0.7.3 compat krusty-only \
        0000000000000000000000000000000000000000000000000000000000000000
    activate_unix_release
    [ "$("$INSTALL_DIR/krusty")" = compat ]
    [ ! -e "$INSTALL_DIR/krusty-mako" ] && [ ! -L "$INSTALL_DIR/krusty-mako" ]
    echo "install.sh self-test passed"
)

install() {
    detect_platform

    VERSION="${VERSION:-$(get_latest_version)}"
    if [ -z "$VERSION" ]; then
        fail "Could not determine latest version."
        exit 1
    fi

    echo "Installing Mitsuro $VERSION for $PLATFORM..."
    ARCHIVE="krusty-$PLATFORM.$EXT"
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$VERSION/$ARCHIVE"
    CHECKSUM_URL="$DOWNLOAD_URL.sha256"
    TMP_DIR="$(mktemp -d)"
    PAYLOAD_DIR="$TMP_DIR/payload"

    echo "Downloading from $DOWNLOAD_URL..."
    curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ARCHIVE"
    echo "Downloading checksum from $CHECKSUM_URL..."
    if ! curl -fsSL "$CHECKSUM_URL" -o "$TMP_DIR/$ARCHIVE.sha256"; then
        fail "Checksum file is required but could not be downloaded."
        exit 1
    fi

    echo "Verifying checksum..."
    if ! verify_download "$TMP_DIR/$ARCHIVE" "$TMP_DIR/$ARCHIVE.sha256" "$ARCHIVE"; then
        exit 1
    fi
    echo "Checksum verified."

    echo "Extracting..."
    extract_archive "$TMP_DIR/$ARCHIVE" "$PAYLOAD_DIR"

    INSTALLED_SYSTEMD_UNITS=false
    if [ "$EXT" = "zip" ]; then
        install_windows_direct "$PAYLOAD_DIR"
    else
        echo "Installing release to $INSTALL_DIR..."
        acquire_install_lock
        stage_unix_release "$PAYLOAD_DIR"
        activate_unix_release
    fi

    echo ""
    echo "Mitsuro installed successfully!"
    if [ "$EXT" != "zip" ]; then
        echo "Active release: $RELEASE_ID"
    fi
    echo ""

    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            echo "Add this to your shell config (.bashrc, .zshrc, etc.):"
            echo ""
            echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
            echo ""
            ;;
    esac

    echo "Run 'krusty' to start."
    if [ "$INSTALLED_SYSTEMD_UNITS" = true ]; then
        echo "To supervise Hive and the self-hosted server:"
        echo "  systemctl --user enable --now krusty-mako.socket krusty-serve.service"
    fi
}

if [ "${1:-}" = "--self-test" ]; then
    run_self_test
else
    install
fi
