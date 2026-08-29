#!/bin/sh
set -e

# Mitsuro installer
# Usage: curl -fsSLO https://raw.githubusercontent.com/honeycomb-Technologies/Mitsuro/main/install.sh && sh install.sh
# Validation: sh install.sh --self-test

REPO="honeycomb-Technologies/Mitsuro"
BINARY="mitsuro"
DAEMON_BINARY="mitsuro-hive"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
SYSTEMD_UNITS="mitsuro-hive.socket mitsuro-hive.service mitsuro-serve.service"
CANONICAL_CONFIG_BASENAME=".mitsuro"
CANONICAL_DATABASE_BASENAME="mitsuro.db"

# Previous-identity compatibility boundary. These names are read only while an
# installed release is upgraded; every new artifact, link, unit, and state path
# written by this installer uses the canonical names above.
LEGACY_BINARY="krusty"
LEGACY_DAEMON_BINARY="krusty-mako"
COMPAT_BINARY="$LEGACY_BINARY"
COMPAT_DAEMON_BINARY="$LEGACY_DAEMON_BINARY"
LEGACY_CONFIG_BASENAME=".krusty"
LEGACY_DATABASE_BASENAME="krusty.db"
LEGACY_HIVE_DIR_BASENAME="mako"
LEGACY_HIVE_KEY_BASENAME="mako-ipc.key"
LEGACY_RUNTIME_BASENAME="krusty"
LEGACY_SOCKET_BASENAME="mako.sock"
LEGACY_CURRENT_BASENAME=".krusty-current"
LEGACY_RELEASES_BASENAME=".krusty-releases"
LEGACY_MARKER_BASENAME=".krusty-systemd-managed"
LEGACY_HIVE_SOCKET_UNIT="krusty-mako.socket"
LEGACY_HIVE_SERVICE_UNIT="krusty-mako.service"
LEGACY_SERVE_UNIT="krusty-serve.service"
LEGACY_SYSTEMD_UNITS="$LEGACY_HIVE_SOCKET_UNIT $LEGACY_HIVE_SERVICE_UNIT $LEGACY_SERVE_UNIT"
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
LEGACY_INSTALL_FOUND=false
LEGACY_ACTIVE_UNITS=""
MIGRATION_TARGET_UNITS=""
CANONICAL_ENABLED_UNITS=""
LEGACY_ENABLED_UNITS=""
MIGRATION_ENABLE_TARGETS=""
NEWLY_ENABLED_CANONICAL_UNITS=""
DISABLED_LEGACY_UNITS=""
STATE_MIGRATION_RECORD=""
STATE_MIGRATION_PENDING=false
STATE_MIGRATION_PERFORMED=false
STATE_MIGRATION_RECEIPTED=false
STATE_SOURCE_DB_DIGEST=""
STATE_SOURCE_WAL_DIGEST=""
SELF_TEST_STATE_MIGRATION=false
WINDOWS_STATE_MIGRATION_REQUIRED=false
WINDOWS_PUBLICATION_IN_PROGRESS=false
WINDOWS_PUBLICATION_BACKUP=""
WINDOWS_CANONICAL_DESTINATION=""
WINDOWS_COMPAT_DESTINATION=""
WINDOWS_CANONICAL_STAGE=""
WINDOWS_COMPAT_STAGE=""
WINDOWS_STAGES_OWNED=false
PROC_ROOT="/proc"
SELF_TEST_PROCFS=false
SELF_TEST_FAIL_WINDOWS_RESTORE=false
MIGRATION_CHILD_PID=""
MIGRATION_CHILD_EXE=""
MIGRATION_EXPECTED_EXE=""

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
    if [ "$WINDOWS_PUBLICATION_IN_PROGRESS" = true ]; then
        rollback_windows_publication >/dev/null 2>&1 || true
    fi
    cleanup_windows_stages
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
    if [ "$WINDOWS_PUBLICATION_IN_PROGRESS" = true ]; then
        rollback_windows_publication || true
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

valid_release_tag() {
    candidate=$1
    case "$candidate" in
        ''|*[!0-9A-Za-z.+-]*) return 1 ;;
    esac
    printf '%s\n' "$candidate" | \
        grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
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
        zip_member=$(unzip -Z1 "$preflight_source") || return 1
        zip_entries=$(printf '%s\n' "$zip_member" | awk 'NF { count++ } END { print count + 0 }')
        [ "$zip_entries" -eq 1 ] || return 1
        [ "$zip_member" = "$BINARY.exe" ] || return 1
        unzip -Z1 "$preflight_source" | safe_member_paths || return 1
        unzip -Z -l "$preflight_source" | safe_zip_member_types "$zip_entries" || return 1
    fi
}

acquire_install_lock() {
    mkdir -p "$INSTALL_DIR"
    INSTALL_LOCK="$INSTALL_DIR/.mitsuro-install.lock"
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
    link_tmp="$link_parent/.$link_leaf.mitsuro-new.$$"

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
    regular_file_with_mode "$candidate_release/mitsuro" 555 || return 1
    regular_file_with_mode "$existing_release/mitsuro" 555 || return 1
    cmp -s "$candidate_release/mitsuro" "$existing_release/mitsuro" || return 1
    regular_file_with_mode "$candidate_release/.archive-sha256" 444 || return 1
    regular_file_with_mode "$existing_release/.archive-sha256" 444 || return 1
    cmp -s "$candidate_release/.archive-sha256" "$existing_release/.archive-sha256" || return 1
    same_optional_file "$candidate_release/mitsuro-hive" "$existing_release/mitsuro-hive" 555 || return 1
    same_optional_file "$candidate_release/agent-browser" "$existing_release/agent-browser" 555 || return 1
    regular_file_with_mode "$candidate_release/$COMPAT_BINARY" 555 || return 1
    regular_file_with_mode "$existing_release/$COMPAT_BINARY" 555 || return 1
    cmp -s "$candidate_release/$COMPAT_BINARY" "$existing_release/$COMPAT_BINARY" || return 1
    same_optional_file "$candidate_release/$COMPAT_DAEMON_BINARY" \
        "$existing_release/$COMPAT_DAEMON_BINARY" 555 || return 1
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
    if [ ! -f "$payload_dir/$COMPAT_BINARY" ] || [ -L "$payload_dir/$COMPAT_BINARY" ]; then
        fail "The bridge release does not contain regular compatibility binary $COMPAT_BINARY."
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
    if [ "$payload_has_daemon" = true ]; then
        if [ ! -f "$payload_dir/$COMPAT_DAEMON_BINARY" ] || \
            [ -L "$payload_dir/$COMPAT_DAEMON_BINARY" ]; then
            fail "The bridge release does not contain regular compatibility binary $COMPAT_DAEMON_BINARY."
            return 1
        fi
    elif [ -e "$payload_dir/$COMPAT_DAEMON_BINARY" ] || \
        [ -L "$payload_dir/$COMPAT_DAEMON_BINARY" ]; then
        fail "$COMPAT_DAEMON_BINARY may be shipped only with $DAEMON_BINARY."
        return 1
    fi

    safe_version=$(printf '%s' "$VERSION" | sed 's/[^A-Za-z0-9._-]/_/g')
    [ -n "$safe_version" ] || safe_version="release"
    RELEASE_ID="$safe_version-$PLATFORM-$ARCHIVE_SHA256"
    RELEASES_DIR="$INSTALL_DIR/.mitsuro-releases"
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
    cp "$payload_dir/$COMPAT_BINARY" "$release_stage/$COMPAT_BINARY"
    chmod 0555 "$release_stage/$COMPAT_BINARY"
    if [ -f "$payload_dir/agent-browser" ] && [ ! -L "$payload_dir/agent-browser" ]; then
        cp "$payload_dir/agent-browser" "$release_stage/agent-browser"
        chmod 0555 "$release_stage/agent-browser"
    elif [ -e "$payload_dir/agent-browser" ] || [ -L "$payload_dir/agent-browser" ]; then
        fail "agent-browser must be a regular file when present."
        return 1
    fi
    if [ "$payload_has_daemon" = true ]; then
        cp "$payload_dir/$COMPAT_DAEMON_BINARY" "$release_stage/$COMPAT_DAEMON_BINARY"
        chmod 0555 "$release_stage/$COMPAT_DAEMON_BINARY"
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

validate_previous_release_pointer() {
    previous_pointer="$INSTALL_DIR/$LEGACY_CURRENT_BASENAME"
    if [ ! -L "$previous_pointer" ]; then
        if [ -e "$previous_pointer" ]; then
            fail "Refusing non-symlink previous release pointer $previous_pointer."
        else
            fail "Previous release pointer is missing: $previous_pointer"
        fi
        return 1
    fi
    previous_pointer_target=$(readlink "$previous_pointer") || return 1
    case "/$previous_pointer_target/" in
        */../*|*/./*)
            fail "Previous release pointer contains an unsafe path: $previous_pointer_target"
            return 1
            ;;
    esac
    case "$previous_pointer_target" in
        "$LEGACY_RELEASES_BASENAME"/*)
            previous_release_path="$INSTALL_DIR/$previous_pointer_target"
            ;;
        "$INSTALL_DIR/$LEGACY_RELEASES_BASENAME"/*)
            previous_release_path="$previous_pointer_target"
            ;;
        *)
            fail "Refusing unmanaged previous release pointer $previous_pointer -> $previous_pointer_target."
            return 1
            ;;
    esac
    if [ ! -d "$previous_release_path" ] || [ -L "$previous_release_path" ]; then
        fail "Previous release pointer does not select a regular directory: $previous_release_path"
        return 1
    fi
    PREVIOUS_IDENTITY_RELEASE_PATH="$previous_release_path"
    PREVIOUS_IDENTITY_RELEASE_TARGET="$previous_pointer_target"
}

resolve_previous_identity_file() {
    previous_candidate=$1
    previous_relative_path=$2
    PREVIOUS_IDENTITY_SOURCE=""
    if [ -L "$previous_candidate" ]; then
        previous_candidate_target=$(readlink "$previous_candidate") || return 1
        case "$previous_candidate_target" in
            "$LEGACY_CURRENT_BASENAME/$previous_relative_path"|\
            "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/$previous_relative_path")
                validate_previous_release_pointer || return 1
                PREVIOUS_IDENTITY_SOURCE="$PREVIOUS_IDENTITY_RELEASE_PATH/$previous_relative_path"
                ;;
            *)
                fail "Refusing unmanaged previous-identity symlink $previous_candidate -> $previous_candidate_target."
                return 1
                ;;
        esac
    elif [ -e "$previous_candidate" ]; then
        if [ ! -f "$previous_candidate" ]; then
            fail "Previous-identity path is not a regular file: $previous_candidate"
            return 1
        fi
        PREVIOUS_IDENTITY_SOURCE="$previous_candidate"
    else
        return 0
    fi

    if [ ! -f "$PREVIOUS_IDENTITY_SOURCE" ] || [ -L "$PREVIOUS_IDENTITY_SOURCE" ]; then
        fail "Previous-identity source is not a regular file: $PREVIOUS_IDENTITY_SOURCE"
        return 1
    fi
}

capture_previous_identity_file() {
    previous_candidate=$1
    previous_relative_path=$2
    previous_destination=$3
    previous_mode=$4
    resolve_previous_identity_file "$previous_candidate" "$previous_relative_path" || return 1
    [ -n "$PREVIOUS_IDENTITY_SOURCE" ] || return 0
    previous_destination_parent=$(dirname "$previous_destination")
    mkdir -p "$previous_destination_parent" || return 1
    cp "$PREVIOUS_IDENTITY_SOURCE" "$previous_destination" || return 1
    chmod "$previous_mode" "$previous_destination" || return 1
    LEGACY_HAS_CONTENT=true
}

create_legacy_release() {
    legacy_include_systemd=$1
    LEGACY_HAS_CONTENT=false
    PREVIOUS_IDENTITY_RELEASE_PATH=""
    PREVIOUS_IDENTITY_RELEASE_TARGET=""
    legacy_stage="$RELEASES_DIR/.legacy-stage-$$"
    RELEASE_STAGE="$legacy_stage"
    mkdir "$legacy_stage"

    copy_legacy_file "$INSTALL_DIR/$BINARY" "$legacy_stage/$BINARY" 0555 || return 1
    copy_legacy_file "$INSTALL_DIR/$DAEMON_BINARY" "$legacy_stage/$DAEMON_BINARY" 0555 || return 1

    # A true previous-generation install publishes only the old command names
    # and may point them through its own immutable release tree. Preserve those
    # bytes under both their historical names and canonical fallback names so
    # the captured release remains directly executable without modifying or
    # moving the old release authority.
    capture_previous_identity_file \
        "$INSTALL_DIR/$LEGACY_BINARY" \
        "$LEGACY_BINARY" \
        "$legacy_stage/$COMPAT_BINARY" 0555 || return 1
    if [ -f "$legacy_stage/$COMPAT_BINARY" ] && [ ! -f "$legacy_stage/$BINARY" ]; then
        cp "$legacy_stage/$COMPAT_BINARY" "$legacy_stage/$BINARY" || return 1
        chmod 0555 "$legacy_stage/$BINARY" || return 1
    fi
    capture_previous_identity_file \
        "$INSTALL_DIR/$LEGACY_DAEMON_BINARY" \
        "$LEGACY_DAEMON_BINARY" \
        "$legacy_stage/$COMPAT_DAEMON_BINARY" 0555 || return 1
    if [ -f "$legacy_stage/$COMPAT_DAEMON_BINARY" ] && \
        [ ! -f "$legacy_stage/$DAEMON_BINARY" ]; then
        cp "$legacy_stage/$COMPAT_DAEMON_BINARY" "$legacy_stage/$DAEMON_BINARY" || return 1
        chmod 0555 "$legacy_stage/$DAEMON_BINARY" || return 1
    fi

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
        for previous_unit in $LEGACY_SYSTEMD_UNITS; do
            previous_unit_source="$legacy_unit_dir/$previous_unit"
            capture_previous_identity_file \
                "$previous_unit_source" \
                "systemd/$previous_unit" \
                "$legacy_stage/previous-systemd/$previous_unit" 0444 || return 1
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
    if [ -d "$legacy_stage/previous-systemd" ]; then
        chmod 0555 "$legacy_stage/previous-systemd"
    fi
    if [ -n "${PREVIOUS_IDENTITY_RELEASE_TARGET:-}" ]; then
        printf '%s\n' "$PREVIOUS_IDENTITY_RELEASE_TARGET" > \
            "$legacy_stage/.previous-release-target"
        chmod 0444 "$legacy_stage/.previous-release-target"
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
    PREVIOUS_TARGET=".mitsuro-releases/$legacy_id"
    atomic_symlink "$PREVIOUS_TARGET" "$CURRENT_LINK"
}

# Normalize a release pointer to the managed relative form:
#   .mitsuro-releases/<id>
# Accepts that relative form, or an absolute path under
# $INSTALL_DIR/.mitsuro-releases/<id> (common after local/dev cuts).
normalize_managed_release_pointer() {
    pointer_target=$1
    case "$pointer_target" in
        .mitsuro-releases/*)
            printf '%s\n' "$pointer_target"
            return 0
            ;;
        "$INSTALL_DIR/.mitsuro-releases"/*)
            relative_id=${pointer_target#"$INSTALL_DIR/.mitsuro-releases/"}
            case "$relative_id" in
                ""|*/*|..|*/../*|.*)
                    return 1
                    ;;
            esac
            printf '%s\n' ".mitsuro-releases/$relative_id"
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

managed_symlink_is_replaceable() {
    existing_target=$1
    managed_target=$2
    previous_managed_target=$3
    alternate_previous_target=$4

    [ "$existing_target" = "$managed_target" ] && return 0
    [ -n "$previous_managed_target" ] && [ "$existing_target" = "$previous_managed_target" ] && return 0
    [ -n "$alternate_previous_target" ] && [ "$existing_target" = "$alternate_previous_target" ] && return 0

    case "$existing_target" in
        .mitsuro-current/*|.mitsuro-releases/*)
            return 0
            ;;
        "$INSTALL_DIR/.mitsuro-current"/*|"$INSTALL_DIR/.mitsuro-releases"/*)
            return 0
            ;;
        "$LEGACY_CURRENT_BASENAME"/*|"$LEGACY_RELEASES_BASENAME"/*)
            return 0
            ;;
        "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME"/*|"$INSTALL_DIR/$LEGACY_RELEASES_BASENAME"/*)
            return 0
            ;;
    esac
    return 1
}

read_previous_release() {
    PREVIOUS_TARGET=""
    if [ -L "$CURRENT_LINK" ]; then
        raw_previous_target="$(readlink "$CURRENT_LINK")"
        if ! PREVIOUS_TARGET=$(normalize_managed_release_pointer "$raw_previous_target"); then
            fail "Refusing unmanaged release pointer $CURRENT_LINK -> $raw_previous_target."
            return 1
        fi
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
    alternate_previous_target=${5:-}

    if [ -L "$managed_path" ]; then
        existing_target=$(readlink "$managed_path")
        if ! managed_symlink_is_replaceable \
            "$existing_target" \
            "$managed_target" \
            "$previous_managed_target" \
            "$alternate_previous_target"; then
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

file_uid() {
    uid_path=$1
    if uid_value=$(stat -c '%u' "$uid_path" 2>/dev/null); then
        printf '%s\n' "$uid_value"
    else
        stat -f '%u' "$uid_path" 2>/dev/null
    fi
}

file_gid() {
    gid_path=$1
    if gid_value=$(stat -c '%g' "$gid_path" 2>/dev/null); then
        printf '%s\n' "$gid_value"
    else
        stat -f '%g' "$gid_path" 2>/dev/null
    fi
}

file_nlink() {
    nlink_path=$1
    if nlink_value=$(stat -c '%h' "$nlink_path" 2>/dev/null); then
        printf '%s\n' "$nlink_value"
    else
        stat -f '%l' "$nlink_path" 2>/dev/null
    fi
}

regular_current_user_file() {
    current_user_path=$1
    current_user_mode=$2
    regular_file_with_mode "$current_user_path" "$current_user_mode" && \
        [ "$(file_uid "$current_user_path")" = "$(id -u)" ] && \
        [ "$(file_gid "$current_user_path")" = "$(id -g)" ] && \
        [ "$(file_nlink "$current_user_path")" = 1 ]
}

exact_unit_line_count() {
    exact_unit_path=$1
    exact_unit_line=$2
    grep -F -x "$exact_unit_line" "$exact_unit_path" 2>/dev/null | \
        wc -l | tr -d '[:space:]'
}

normalize_adoptable_serve_unit() {
    adoptable_unit_input=$1
    adoptable_unit_output=$2
    adoptable_unit_kind=$3
    : > "$adoptable_unit_output" || return 1
    while IFS= read -r adoptable_unit_line || [ -n "$adoptable_unit_line" ]; do
        if [ "$adoptable_unit_line" = \
            "ExecStart=$INSTALL_DIR/.mitsuro-current/$BINARY serve --port 3000" ] || \
            [ "$adoptable_unit_line" = \
            "ExecStart=%h/.local/bin/.mitsuro-current/$BINARY serve --port 3000" ]; then
            printf '%s\n' 'ExecStart=@mitsuro-current@/mitsuro serve --port 3000'
        elif [ "$adoptable_unit_kind" = candidate ] && \
            [ "$adoptable_unit_line" = \
            'Environment=MITSURO_AGENT_BROWSER_PATH=%h/.local/bin/.mitsuro-current/agent-browser' ]; then
            # The Atlas sidecar path was added after the first canonical user
            # unit was installed. Only the candidate may drop this one line
            # for the predecessor comparison.
            :
        else
            printf '%s\n' "$adoptable_unit_line"
        fi
    done < "$adoptable_unit_input" > "$adoptable_unit_output"
}

legacy_serve_unit_matches_candidate() {
    legacy_serve_existing=$1
    legacy_serve_candidate=$2
    [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ] || return 1
    legacy_absolute_exec="ExecStart=$INSTALL_DIR/.mitsuro-current/$BINARY serve --port 3000"
    legacy_home_exec="ExecStart=%h/.local/bin/.mitsuro-current/$BINARY serve --port 3000"
    candidate_atlas='Environment=MITSURO_AGENT_BROWSER_PATH=%h/.local/bin/.mitsuro-current/agent-browser'
    legacy_exec_count=$((
        $(exact_unit_line_count "$legacy_serve_existing" "$legacy_absolute_exec") +
        $(exact_unit_line_count "$legacy_serve_existing" "$legacy_home_exec")
    ))
    [ "$legacy_exec_count" = 1 ] || return 1
    [ "$(exact_unit_line_count "$legacy_serve_existing" "$candidate_atlas")" = 0 ] || return 1
    [ "$(exact_unit_line_count "$legacy_serve_candidate" "$legacy_home_exec")" = 1 ] || return 1
    [ "$(exact_unit_line_count "$legacy_serve_candidate" "$candidate_atlas")" = 1 ] || return 1
    legacy_serve_existing_normalized="$ACTIVATION_BACKUP/adopt-existing-serve"
    legacy_serve_candidate_normalized="$ACTIVATION_BACKUP/adopt-candidate-serve"
    normalize_adoptable_serve_unit \
        "$legacy_serve_existing" "$legacy_serve_existing_normalized" existing || return 1
    normalize_adoptable_serve_unit \
        "$legacy_serve_candidate" "$legacy_serve_candidate_normalized" candidate || return 1
    cmp -s "$legacy_serve_existing_normalized" "$legacy_serve_candidate_normalized"
}

unmarked_canonical_units_adoptable() {
    [ -n "$PREVIOUS_TARGET" ] || return 1
    [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ] || return 1
    [ ! -e "$SYSTEMD_MARKER" ] && [ ! -L "$SYSTEMD_MARKER" ] || return 1
    for adoptable_unit in $SYSTEMD_UNITS; do
        adoptable_existing="$SYSTEMD_USER_DIR/$adoptable_unit"
        adoptable_candidate="$RELEASE_DIR/systemd/$adoptable_unit"
        regular_current_user_file "$adoptable_existing" 644 || return 1
        regular_file_with_mode "$adoptable_candidate" 444 || return 1
        if cmp -s "$adoptable_existing" "$adoptable_candidate"; then
            continue
        fi
        [ "$adoptable_unit" = mitsuro-serve.service ] || return 1
        legacy_serve_unit_matches_candidate \
            "$adoptable_existing" "$adoptable_candidate" || return 1
    done
}

canonical_unit_unchanged_since_snapshot() {
    unchanged_unit=$1
    case "$unchanged_unit" in
        mitsuro-hive.socket) unchanged_key=hive-socket ;;
        mitsuro-hive.service) unchanged_key=hive-service ;;
        mitsuro-serve.service) unchanged_key=serve-service ;;
        *) return 1 ;;
    esac
    unchanged_path="$SYSTEMD_USER_DIR/$unchanged_unit"
    regular_current_user_file "$unchanged_path" 644 && \
        [ "$(sed -n '1p' "$ACTIVATION_BACKUP/$unchanged_key.state")" = file ] && \
        cmp -s "$unchanged_path" "$ACTIVATION_BACKUP/$unchanged_key.file"
}

canonical_regular_systemd_unit_present() {
    for canonical_regular_unit in $SYSTEMD_UNITS; do
        if [ -f "$SYSTEMD_USER_DIR/$canonical_regular_unit" ] && \
            [ ! -L "$SYSTEMD_USER_DIR/$canonical_regular_unit" ]; then
            return 0
        fi
    done
    return 1
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
            restore_tmp="$restore_parent/.$restore_leaf.mitsuro-restore.$$"
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
            restore_tmp="$restore_parent/.$restore_leaf.mitsuro-restore.$$"
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
    ACTIVATION_BACKUP="$INSTALL_DIR/.mitsuro-activation-backup.$$"
    if [ -e "$ACTIVATION_BACKUP" ] || [ -L "$ACTIVATION_BACKUP" ]; then
        fail "Unexpected activation backup already exists: $ACTIVATION_BACKUP"
        return 1
    fi
    mkdir "$ACTIVATION_BACKUP"
    chmod 0700 "$ACTIVATION_BACKUP"
    SYSTEMD_USER_DIR_WAS_PRESENT=false
    [ -d "$SYSTEMD_USER_DIR" ] && SYSTEMD_USER_DIR_WAS_PRESENT=true
    snapshot_activation_path current "$CURRENT_LINK" || return 1
    snapshot_activation_path mitsuro "$INSTALL_DIR/$BINARY" || return 1
    snapshot_activation_path hive "$INSTALL_DIR/$DAEMON_BINARY" || return 1
    snapshot_activation_path atlas "$INSTALL_DIR/agent-browser" || return 1
    snapshot_activation_path compat-cli "$INSTALL_DIR/$COMPAT_BINARY" || return 1
    snapshot_activation_path compat-hive "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" || return 1
    if [ "$MANAGE_SYSTEMD" = true ]; then
        snapshot_activation_path marker "$SYSTEMD_MARKER" || return 1
        for snapshot_unit in $SYSTEMD_UNITS; do
            case "$snapshot_unit" in
                mitsuro-hive.socket) snapshot_key=hive-socket ;;
                mitsuro-hive.service) snapshot_key=hive-service ;;
                mitsuro-serve.service) snapshot_key=serve-service ;;
            esac
            snapshot_activation_path "$snapshot_key" "$SYSTEMD_USER_DIR/$snapshot_unit" || return 1
        done
    fi
}

restore_activation_snapshot() {
    restore_failed=false
    restore_activation_path current "$CURRENT_LINK" || restore_failed=true
    restore_activation_path mitsuro "$INSTALL_DIR/$BINARY" || restore_failed=true
    restore_activation_path hive "$INSTALL_DIR/$DAEMON_BINARY" || restore_failed=true
    restore_activation_path atlas "$INSTALL_DIR/agent-browser" || restore_failed=true
    restore_activation_path compat-cli "$INSTALL_DIR/$COMPAT_BINARY" || restore_failed=true
    restore_activation_path compat-hive "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" || restore_failed=true
    if [ "$MANAGE_SYSTEMD" = true ]; then
        restore_activation_path marker "$SYSTEMD_MARKER" || restore_failed=true
        for restore_unit in $SYSTEMD_UNITS; do
            case "$restore_unit" in
                mitsuro-hive.socket) restore_key=hive-socket ;;
                mitsuro-hive.service) restore_key=hive-service ;;
                mitsuro-serve.service) restore_key=serve-service ;;
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
    marker_tmp="$marker_parent/.$marker_leaf.mitsuro-new.$$"
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

canonical_unit_for_legacy() {
    case "$1" in
        "$LEGACY_HIVE_SOCKET_UNIT") printf '%s\n' "mitsuro-hive.socket" ;;
        "$LEGACY_HIVE_SERVICE_UNIT") printf '%s\n' "mitsuro-hive.service" ;;
        "$LEGACY_SERVE_UNIT") printf '%s\n' "mitsuro-serve.service" ;;
        *) return 1 ;;
    esac
}

detect_legacy_installation() {
    LEGACY_INSTALL_FOUND=false
    for legacy_install_path in \
        "$INSTALL_DIR/$LEGACY_BINARY" \
        "$INSTALL_DIR/$LEGACY_DAEMON_BINARY" \
        "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME" \
        "$INSTALL_DIR/$LEGACY_RELEASES_BASENAME" \
        "$INSTALL_DIR/$LEGACY_MARKER_BASENAME"; do
        if [ -e "$legacy_install_path" ] || [ -L "$legacy_install_path" ]; then
            LEGACY_INSTALL_FOUND=true
            break
        fi
    done
    if [ "$OS" = "Linux" ] && [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ]; then
        for legacy_install_unit in $LEGACY_SYSTEMD_UNITS; do
            if [ -e "$SYSTEMD_USER_DIR/$legacy_install_unit" ] || \
                [ -L "$SYSTEMD_USER_DIR/$legacy_install_unit" ]; then
                LEGACY_INSTALL_FOUND=true
                break
            fi
        done
    fi
}

mitsuro_state_executable_path() {
    inspected_executable=$1
    case "$inspected_executable" in
        *"/$LEGACY_RELEASES_BASENAME/"*|*/.mitsuro-releases/*) return 0 ;;
    esac
    inspected_basename=$(basename "$inspected_executable") || return 1
    case "$inspected_basename" in
        mitsuro|mitsuro.exe|mitsuro-hive|mitsuro-hive.exe|\
        mitsuro-desktop|mitsuro-desktop.exe|Mitsuro|Mitsuro.exe|\
        "$LEGACY_BINARY"|"$LEGACY_BINARY.exe"|\
        "$LEGACY_DAEMON_BINARY"|"$LEGACY_DAEMON_BINARY.exe"|\
        krusty-desktop|krusty-desktop.exe|Krusty|Krusty.exe)
            return 0
            ;;
        *) return 1 ;;
    esac
}

identity_database_fd_target() {
    inspected_fd_target=$1
    case "$inspected_fd_target" in
        *" (deleted)") inspected_fd_target=${inspected_fd_target%" (deleted)"} ;;
    esac
    case "$inspected_fd_target" in
        "$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME"|\
        "$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME-wal"|\
        "$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME-shm"|\
        "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME"|\
        "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME-wal"|\
        "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME-shm")
            return 0
            ;;
        *) return 1 ;;
    esac
}

migration_child_is_proven() {
    inspected_pid=$1
    inspected_executable=$2
    [ -n "$MIGRATION_CHILD_PID" ] && \
        [ "$inspected_pid" = "$MIGRATION_CHILD_PID" ] && \
        [ -n "$MIGRATION_CHILD_EXE" ] && \
        [ "$inspected_executable" = "$MIGRATION_CHILD_EXE" ] && \
        [ -n "$MIGRATION_EXPECTED_EXE" ] && \
        [ "$MIGRATION_CHILD_EXE" = "$MIGRATION_EXPECTED_EXE" ]
}

prove_legacy_state_quiescent() {
    quiescence_phase=$1
    [ "$STATE_MIGRATION_PENDING" = true ] || return 0
    if [ "$OS" != Linux ]; then
        fail "Offline identity migration cannot prove every Mitsuro process is stopped on $OS. Stop every Mitsuro CLI, TUI, desktop, server, and Hive process, then run '$RELEASE_DIR/$BINARY migrate-identity --confirm-offline' manually before retrying the installer."
        return 1
    fi
    if [ ! -d "$PROC_ROOT" ]; then
        fail "Cannot prove offline identity migration authority: $PROC_ROOT is unavailable ($quiescence_phase)."
        return 1
    fi
    if [ "$SELF_TEST_PROCFS" != true ]; then
        if [ "$PROC_ROOT" != /proc ] || [ ! -r "$PROC_ROOT/$$/status" ] || \
            [ ! -d "$PROC_ROOT/$$/fd" ]; then
            fail "Cannot inspect the installer's own procfs entry; refusing identity migration ($quiescence_phase)."
            return 1
        fi
    fi
    quiescence_uid=$(id -u 2>/dev/null) || {
        fail "Cannot determine the current user for identity migration ($quiescence_phase)."
        return 1
    }

    for inspected_process_dir in "$PROC_ROOT"/[0-9]*; do
        [ -d "$inspected_process_dir" ] || continue
        inspected_pid=${inspected_process_dir##*/}
        inspected_status="$inspected_process_dir/status"
        inspected_uid=$(awk '/^Uid:/ { print $2; found = 1; exit } END { if (!found) exit 1 }' \
            "$inspected_status" 2>/dev/null) || {
            [ ! -d "$inspected_process_dir" ] && continue
            fail "Cannot inspect same-user process $inspected_pid during identity migration ($quiescence_phase)."
            return 1
        }
        [ "$inspected_uid" = "$quiescence_uid" ] || continue
        inspected_state=$(awk '/^State:/ { print $2; found = 1; exit } END { if (!found) exit 1 }' \
            "$inspected_status" 2>/dev/null) || {
            [ ! -d "$inspected_process_dir" ] && continue
            fail "Cannot inspect process state for PID $inspected_pid ($quiescence_phase)."
            return 1
        }
        [ "$inspected_state" = Z ] && continue

        inspected_executable=$(readlink "$inspected_process_dir/exe" 2>/dev/null) || {
            [ ! -d "$inspected_process_dir" ] && continue
            fail "Cannot resolve the executable for same-user PID $inspected_pid ($quiescence_phase)."
            return 1
        }
        case "$inspected_executable" in
            *" (deleted)") inspected_executable=${inspected_executable%" (deleted)"} ;;
        esac
        if migration_child_is_proven "$inspected_pid" "$inspected_executable"; then
            continue
        fi
        if mitsuro_state_executable_path "$inspected_executable"; then
            fail "Mitsuro process PID $inspected_pid is still running from $inspected_executable ($quiescence_phase). Stop every Mitsuro CLI, TUI, desktop, server, and Hive process before migrating state."
            return 1
        fi

        inspected_fd_dir="$inspected_process_dir/fd"
        if [ ! -d "$inspected_fd_dir" ] || [ ! -r "$inspected_fd_dir" ]; then
            [ ! -d "$inspected_process_dir" ] && continue
            fail "Cannot inspect file descriptors for same-user PID $inspected_pid ($quiescence_phase)."
            return 1
        fi
        for inspected_fd in "$inspected_fd_dir"/*; do
            if [ ! -e "$inspected_fd" ] && [ ! -L "$inspected_fd" ]; then
                continue
            fi
            inspected_fd_target=$(readlink "$inspected_fd" 2>/dev/null) || {
                [ ! -e "$inspected_fd" ] && [ ! -L "$inspected_fd" ] && continue
                fail "Cannot inspect file descriptor $inspected_fd for identity migration ($quiescence_phase)."
                return 1
            }
            if identity_database_fd_target "$inspected_fd_target"; then
                fail "Mitsuro process PID $inspected_pid still has identity state open at $inspected_fd_target ($quiescence_phase). Stop every Mitsuro CLI, TUI, desktop, server, and Hive process before migrating state."
                return 1
            fi
        done
    done
}

preflight_identity_state_roots() {
    CANONICAL_STATE_ROOT="$HOME/$CANONICAL_CONFIG_BASENAME"
    LEGACY_STATE_ROOT="$HOME/$LEGACY_CONFIG_BASENAME"
    STATE_MIGRATION_PENDING=false
    STATE_MIGRATION_PERFORMED=false
    STATE_MIGRATION_RECEIPTED=false
    STATE_MIGRATION_RECORD=""
    STATE_SOURCE_DB_DIGEST=""
    STATE_SOURCE_WAL_DIGEST=""

    for inspected_state_root in "$CANONICAL_STATE_ROOT" "$LEGACY_STATE_ROOT"; do
        if [ -L "$inspected_state_root" ] || \
            { [ -e "$inspected_state_root" ] && [ ! -d "$inspected_state_root" ]; }; then
            fail "Refusing identity migration through non-directory state root $inspected_state_root."
            return 1
        fi
    done

    if [ -d "$CANONICAL_STATE_ROOT" ] && [ -d "$LEGACY_STATE_ROOT" ]; then
        if ! identity_migration_receipt_valid; then
            fail "Canonical and previous Mitsuro roots coexist without a valid migration receipt."
            return 1
        fi
        STATE_MIGRATION_RECEIPTED=true
    fi
    if [ -d "$LEGACY_STATE_ROOT" ] && [ ! -d "$CANONICAL_STATE_ROOT" ]; then
        STATE_MIGRATION_PENDING=true
    fi
}

decimal_within_unsigned_limit() {
    decimal_candidate=$1
    decimal_limit=$2
    case "$decimal_candidate" in
        ""|*[!0-9]*) return 1 ;;
    esac
    decimal_normalized=$(printf '%s\n' "$decimal_candidate" | sed 's/^0*//') || return 1
    [ -n "$decimal_normalized" ] || decimal_normalized=0
    decimal_candidate_length=${#decimal_normalized}
    decimal_limit_length=${#decimal_limit}
    if [ "$decimal_candidate_length" -lt "$decimal_limit_length" ]; then
        return 0
    fi
    if [ "$decimal_candidate_length" -gt "$decimal_limit_length" ]; then
        return 1
    fi
    decimal_max=$(printf '%s\n%s\n' "$decimal_normalized" "$decimal_limit" | \
        LC_ALL=C sort | tail -n 1) || return 1
    [ "$decimal_max" = "$decimal_limit" ]
}

lowercase_sha256_valid() {
    printf '%s\n' "$1" | LC_ALL=C grep -Eq '^[0-9a-f]{64}$'
}

identity_migration_receipt_valid() {
    identity_receipt="$CANONICAL_STATE_ROOT/.identity-migration-v2"
    [ -f "$identity_receipt" ] && [ ! -L "$identity_receipt" ] || return 1
    identity_receipt_size=$(wc -c < "$identity_receipt" | tr -d '[:space:]') || return 1
    case "$identity_receipt_size" in
        ""|*[!0-9]*) return 1 ;;
    esac
    [ "$identity_receipt_size" -le 16384 ] || return 1
    [ "$(wc -l < "$identity_receipt" | tr -d '[:space:]')" = 5 ] || return 1
    identity_last_byte=$(tail -c 1 "$identity_receipt" | od -An -t x1 | \
        tr -d '[:space:]') || return 1
    [ "$identity_last_byte" = 0a ] || return 1
    if LC_ALL=C grep -q "$(printf '\r')" "$identity_receipt"; then
        return 1
    fi

    sed -n '1p' "$identity_receipt" | grep -Fqx 'version=2' || return 1
    sed -n '2p' "$identity_receipt" | grep -Fqx "source=$LEGACY_STATE_ROOT" || return 1
    identity_created_line=$(sed -n '3p' "$identity_receipt") || return 1
    case "$identity_created_line" in
        created_unix=*) identity_created=${identity_created_line#created_unix=} ;;
        *) return 1 ;;
    esac
    decimal_within_unsigned_limit "$identity_created" 18446744073709551615 || return 1
    sed -n '4p' "$identity_receipt" | grep -Fqx 'rollback_preserved=true' || return 1

    identity_authority_line=$(sed -n '5p' "$identity_receipt") || return 1
    case "$identity_authority_line" in
        source_authority_fingerprint=*)
            identity_authority=${identity_authority_line#source_authority_fingerprint=}
            ;;
        *) return 1 ;;
    esac
    case "$identity_authority" in
        sqlite=*'|tree_sha256='*'|tree_stat_sha256='*) ;;
        *) return 1 ;;
    esac
    identity_sqlite_and_trees=${identity_authority#sqlite=}
    identity_sqlite=${identity_sqlite_and_trees%%|tree_sha256=*}
    identity_tree_values=${identity_sqlite_and_trees#*|tree_sha256=}
    [ "$identity_tree_values" != "$identity_sqlite_and_trees" ] || return 1
    identity_tree_sha=${identity_tree_values%%|tree_stat_sha256=*}
    identity_tree_stat=${identity_tree_values#*|tree_stat_sha256=}
    [ "$identity_tree_stat" != "$identity_tree_values" ] || return 1
    lowercase_sha256_valid "$identity_tree_sha" || return 1
    lowercase_sha256_valid "$identity_tree_stat" || return 1

    if [ "$identity_sqlite" = absent ]; then
        return 0
    fi
    identity_sqlite_digest=${identity_sqlite%%;*}
    identity_sqlite_fields=${identity_sqlite#*;}
    [ "$identity_sqlite_fields" != "$identity_sqlite" ] || return 1
    identity_main_len_field=${identity_sqlite_fields%%;*}
    identity_sqlite_fields=${identity_sqlite_fields#*;}
    identity_main_mtime_field=${identity_sqlite_fields%%;*}
    identity_sqlite_fields=${identity_sqlite_fields#*;}
    identity_wal_len_field=${identity_sqlite_fields%%;*}
    identity_wal_mtime_field=${identity_sqlite_fields#*;}
    [ "$identity_wal_mtime_field" != "$identity_sqlite_fields" ] || return 1
    case "$identity_wal_mtime_field" in
        *';'*) return 1 ;;
    esac
    lowercase_sha256_valid "$identity_sqlite_digest" || return 1
    case "$identity_main_len_field" in
        main_len=*) identity_main_len=${identity_main_len_field#main_len=} ;;
        *) return 1 ;;
    esac
    case "$identity_main_mtime_field" in
        main_mtime_ns=*) identity_main_mtime=${identity_main_mtime_field#main_mtime_ns=} ;;
        *) return 1 ;;
    esac
    case "$identity_wal_len_field" in
        wal_len=*) identity_wal_len=${identity_wal_len_field#wal_len=} ;;
        *) return 1 ;;
    esac
    case "$identity_wal_mtime_field" in
        wal_mtime_ns=*) identity_wal_mtime=${identity_wal_mtime_field#wal_mtime_ns=} ;;
        *) return 1 ;;
    esac
    decimal_within_unsigned_limit "$identity_main_len" 18446744073709551615 || return 1
    decimal_within_unsigned_limit \
        "$identity_main_mtime" 340282366920938463463374607431768211455 || return 1
    case "$identity_wal_len:$identity_wal_mtime" in
        absent:absent) return 0 ;;
        absent:*|*:absent) return 1 ;;
    esac
    decimal_within_unsigned_limit "$identity_wal_len" 18446744073709551615 || return 1
    decimal_within_unsigned_limit \
        "$identity_wal_mtime" 340282366920938463463374607431768211455
}

sqlite_integrity_check() {
    checked_database=$1
    checked_label=$2
    if [ ! -e "$checked_database" ] && [ ! -L "$checked_database" ]; then
        return 0
    fi
    if [ ! -f "$checked_database" ] || [ -L "$checked_database" ]; then
        fail "$checked_label database is not a regular file: $checked_database"
        return 1
    fi

    sqlite_check_output=""
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite_check_source="$checked_database"
        if [ ! -e "$checked_database-wal" ] && [ ! -L "$checked_database-wal" ]; then
            # Without a WAL, immutable read-only mode avoids creating empty
            # WAL/SHM sidecars merely to perform this auxiliary check.
            sqlite_check_source="file:$checked_database?immutable=1"
        fi
        sqlite_check_output=$(sqlite3 -readonly "$sqlite_check_source" \
            'PRAGMA quick_check; PRAGMA foreign_key_check;' 2>/dev/null) || {
            fail "$checked_label database integrity command failed: $checked_database"
            return 1
        }
    elif command -v python3 >/dev/null 2>&1; then
        sqlite_check_output=$(python3 - "$checked_database" <<'PY'
import os
import sqlite3
import sys
from urllib.parse import quote

database = sys.argv[1]
parameters = "mode=ro"
if not os.path.lexists(f"{database}-wal"):
    parameters += "&immutable=1"
uri = f"file:{quote(os.path.abspath(database), safe='/')}?{parameters}"
connection = sqlite3.connect(uri, uri=True)
try:
    quick = [row[0] for row in connection.execute("PRAGMA quick_check")]
    foreign_keys = list(connection.execute("PRAGMA foreign_key_check"))
finally:
    connection.close()
if quick != ["ok"] or foreign_keys:
    raise SystemExit(1)
print("ok")
PY
        ) || {
            fail "$checked_label database integrity command failed: $checked_database"
            return 1
        }
    else
        echo "Skipping auxiliary $checked_label SQLite check: sqlite3/python3 is unavailable; the canonical migration binary remains authoritative." >&2
        return 0
    fi

    if [ "$sqlite_check_output" != "ok" ]; then
        fail "$checked_label database failed quick_check or foreign_key_check: $checked_database"
        return 1
    fi
}

sqlite_authority_digest() {
    digest_path=$1
    digest_label=$2
    if [ ! -e "$digest_path" ] && [ ! -L "$digest_path" ]; then
        printf '%s\n' absent
        return 0
    fi
    if [ ! -f "$digest_path" ] || [ -L "$digest_path" ]; then
        fail "$digest_label is not a regular file: $digest_path"
        return 1
    fi
    digest_value=$(sha256_file "$digest_path") || return 1
    printf 'sha256:%s\n' "$digest_value"
}

verify_legacy_sqlite_unchanged() {
    [ "$STATE_MIGRATION_PENDING" = true ] || return 0
    verify_legacy_database="$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME"
    verify_db_digest=$(sqlite_authority_digest \
        "$verify_legacy_database" "previous-generation database") || return 1
    verify_wal_digest=$(sqlite_authority_digest \
        "$verify_legacy_database-wal" "previous-generation WAL") || return 1
    if [ "$verify_db_digest" != "$STATE_SOURCE_DB_DIGEST" ] || \
        [ "$verify_wal_digest" != "$STATE_SOURCE_WAL_DIGEST" ]; then
        fail "Previous-generation SQLite database or WAL changed during identity migration."
        return 1
    fi
}

prepare_state_migration_manifest() {
    [ "$STATE_MIGRATION_PENDING" = true ] || return 0
    prove_legacy_state_quiescent "before reading previous SQLite authority" || return 1

    legacy_database="$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME"
    STATE_SOURCE_DB_DIGEST=$(sqlite_authority_digest \
        "$legacy_database" "previous-generation database") || return 1
    STATE_SOURCE_WAL_DIGEST=$(sqlite_authority_digest \
        "$legacy_database-wal" "previous-generation WAL") || return 1
    if [ -L "$legacy_database-shm" ]; then
        fail "previous-generation SHM is a symlink: $legacy_database-shm"
        return 1
    fi
    sqlite_integrity_check "$legacy_database" "previous-generation" || return 1
    verify_legacy_sqlite_unchanged || return 1

    record_root="$INSTALL_DIR/.mitsuro-migration-records"
    mkdir -p "$record_root" || return 1
    chmod 0700 "$record_root" || return 1
    record_base="identity-$(date +%Y%m%d%H%M%S 2>/dev/null || printf '%s' "$$")-$$"
    STATE_MIGRATION_RECORD="$record_root/$record_base"
    record_suffix=0
    while [ -e "$STATE_MIGRATION_RECORD" ] || [ -L "$STATE_MIGRATION_RECORD" ]; do
        record_suffix=$((record_suffix + 1))
        STATE_MIGRATION_RECORD="$record_root/$record_base-$record_suffix"
    done
    mkdir "$STATE_MIGRATION_RECORD" || return 1
    chmod 0700 "$STATE_MIGRATION_RECORD" || return 1

    state_manifest="$STATE_MIGRATION_RECORD/source-manifest"
    {
        printf 'version=1\n'
        printf 'source=%s\n' "$LEGACY_STATE_ROOT"
        printf 'target=%s\n' "$CANONICAL_STATE_ROOT"
        printf 'database=%s\n' "$STATE_SOURCE_DB_DIGEST"
        printf 'wal=%s\n' "$STATE_SOURCE_WAL_DIGEST"
        printf 'shm=ephemeral-not-hashed\n'
    } > "$state_manifest" || return 1
    chmod 0600 "$state_manifest"
}

invoke_canonical_state_migration() {
    [ "$STATE_MIGRATION_PENDING" = true ] || return 0
    migration_binary="$RELEASE_DIR/$BINARY"
    if [ ! -x "$migration_binary" ]; then
        fail "Canonical migration binary is not executable: $migration_binary"
        return 1
    fi

    prove_legacy_state_quiescent "immediately before canonical migration" || return 1

    migration_succeeded=false
    MIGRATION_CHILD_PID=""
    MIGRATION_CHILD_EXE=""
    MIGRATION_EXPECTED_EXE=""
    if [ "$SELF_TEST_STATE_MIGRATION" = true ]; then
        MITSURO_INSTALL_TEST_SOURCE_ROOT="$LEGACY_STATE_ROOT" \
        MITSURO_INSTALL_TEST_TARGET_ROOT="$CANONICAL_STATE_ROOT" \
        MITSURO_INSTALL_TEST_SOURCE_DB="$LEGACY_DATABASE_BASENAME" \
        MITSURO_INSTALL_TEST_TARGET_DB="$CANONICAL_DATABASE_BASENAME" \
        "$migration_binary" migrate-identity --confirm-offline >/dev/null &
    else
        "$migration_binary" migrate-identity --confirm-offline >/dev/null &
    fi
    migration_pid=$!
    migration_expected_dir=$(CDPATH= cd -- "$(dirname "$migration_binary")" && pwd -P) || {
        kill "$migration_pid" 2>/dev/null || true
        wait "$migration_pid" 2>/dev/null || true
        return 1
    }
    migration_expected_exe="$migration_expected_dir/$(basename "$migration_binary")"
    MIGRATION_EXPECTED_EXE=$migration_expected_exe
    MIGRATION_CHILD_PID=$migration_pid
    if [ "$OS" = Linux ] && [ "$PROC_ROOT" = /proc ] && \
        [ -d "$PROC_ROOT/$migration_pid" ]; then
        if migration_observed_exe=$(readlink "$PROC_ROOT/$migration_pid/exe" 2>/dev/null); then
            if [ "$migration_observed_exe" != "$migration_expected_exe" ]; then
                kill "$migration_pid" 2>/dev/null || true
                wait "$migration_pid" 2>/dev/null || true
                MIGRATION_CHILD_PID=""
                MIGRATION_EXPECTED_EXE=""
                fail "Identity migration child executed an unexpected binary: $migration_observed_exe"
                return 1
            fi
            MIGRATION_CHILD_EXE=$migration_observed_exe
        else
            migration_child_state=$(awk '/^State:/ { print $2; exit }' \
                "$PROC_ROOT/$migration_pid/status" 2>/dev/null || true)
            if [ "$migration_child_state" != Z ]; then
                kill "$migration_pid" 2>/dev/null || true
                wait "$migration_pid" 2>/dev/null || true
                MIGRATION_CHILD_PID=""
                MIGRATION_EXPECTED_EXE=""
                fail "Could not prove the canonical migration child executable."
                return 1
            fi
        fi
    fi
    if ! prove_legacy_state_quiescent "while the proven canonical migration child runs"; then
        kill "$migration_pid" 2>/dev/null || true
        wait "$migration_pid" 2>/dev/null || true
        MIGRATION_CHILD_PID=""
        MIGRATION_CHILD_EXE=""
        MIGRATION_EXPECTED_EXE=""
        quarantine_failed_canonical_state || true
        return 1
    fi
    if wait "$migration_pid"; then
        migration_succeeded=true
    fi
    MIGRATION_CHILD_PID=""
    MIGRATION_CHILD_EXE=""
    MIGRATION_EXPECTED_EXE=""
    if ! prove_legacy_state_quiescent "immediately after canonical migration"; then
        quarantine_failed_canonical_state || true
        return 1
    fi
    if ! verify_legacy_sqlite_unchanged; then
        quarantine_failed_canonical_state || true
        fail "Offline identity migration changed the previous rollback authority."
        return 1
    fi
    if [ "$migration_succeeded" != true ]; then
        quarantine_failed_canonical_state || true
        fail "Explicit offline identity migration failed; previous state remains authoritative."
        return 1
    fi
    if [ ! -d "$LEGACY_STATE_ROOT" ] || [ ! -d "$CANONICAL_STATE_ROOT" ] || \
        ! identity_migration_receipt_valid; then
        quarantine_failed_canonical_state || true
        fail "Offline identity migration did not publish a valid canonical state receipt."
        return 1
    fi
    STATE_MIGRATION_PERFORMED=true
    sqlite_integrity_check \
        "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME" \
        "canonical migrated" || return 1
}

quarantine_failed_canonical_state() {
    if [ ! -e "$CANONICAL_STATE_ROOT" ] && [ ! -L "$CANONICAL_STATE_ROOT" ]; then
        return 0
    fi
    failed_state_base="$CANONICAL_STATE_ROOT.failed.$(date +%s 2>/dev/null || printf '%s' "$$").$$"
    failed_state="$failed_state_base"
    failed_state_suffix=0
    while [ -e "$failed_state" ] || [ -L "$failed_state" ]; do
        failed_state_suffix=$((failed_state_suffix + 1))
        failed_state="$failed_state_base.$failed_state_suffix"
    done
    mv "$CANONICAL_STATE_ROOT" "$failed_state" || return 1
    echo "Preserved failed canonical state at $failed_state" >&2
}

validate_canonical_state_after_start() {
    [ "$STATE_MIGRATION_PERFORMED" = true ] || return 0
    sqlite_integrity_check \
        "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME" \
        "canonical post-start" || return 1
}

restore_state_checkpoint() {
    [ "$STATE_MIGRATION_PERFORMED" = true ] || return 0
    if [ -z "$STATE_MIGRATION_RECORD" ] || \
        [ ! -f "$STATE_MIGRATION_RECORD/source-manifest" ] || \
        [ -L "$STATE_MIGRATION_RECORD/source-manifest" ]; then
        fail "State migration source manifest is unavailable."
        return 1
    fi
    if [ ! -d "$LEGACY_STATE_ROOT" ] || [ -L "$LEGACY_STATE_ROOT" ]; then
        fail "Previous state root is unavailable during rollback."
        return 1
    fi
    quarantine_failed_canonical_state || return 1
    verify_legacy_sqlite_unchanged || return 1
    sqlite_integrity_check \
        "$LEGACY_STATE_ROOT/$LEGACY_DATABASE_BASENAME" \
        "preserved previous-generation" || return 1
    STATE_MIGRATION_PERFORMED=false
}

capture_active_services() {
    ACTIVE_UNITS=""
    LEGACY_ACTIVE_UNITS=""
    MIGRATION_TARGET_UNITS=""
    CANONICAL_ENABLED_UNITS=""
    LEGACY_ENABLED_UNITS=""
    MIGRATION_ENABLE_TARGETS=""
    NEWLY_ENABLED_CANONICAL_UNITS=""
    DISABLED_LEGACY_UNITS=""
    if [ "$MANAGE_SYSTEMD" != true ] || ! systemctl_available; then
        return 0
    fi
    for active_unit in $SYSTEMD_UNITS; do
        if run_systemctl --user is-active --quiet "$active_unit" >/dev/null 2>&1; then
            ACTIVE_UNITS="$ACTIVE_UNITS $active_unit"
        fi
        if run_systemctl --user is-enabled --quiet "$active_unit" >/dev/null 2>&1; then
            CANONICAL_ENABLED_UNITS="$CANONICAL_ENABLED_UNITS $active_unit"
        fi
    done
    if [ "$LEGACY_INSTALL_FOUND" = true ]; then
        for legacy_active_unit in $LEGACY_SYSTEMD_UNITS; do
            if run_systemctl --user is-active --quiet "$legacy_active_unit" >/dev/null 2>&1; then
                legacy_target_unit=$(canonical_unit_for_legacy "$legacy_active_unit") || return 1
                LEGACY_ACTIVE_UNITS="$LEGACY_ACTIVE_UNITS $legacy_active_unit"
                case " $ACTIVE_UNITS $MIGRATION_TARGET_UNITS " in
                    *" $legacy_target_unit "*) ;;
                    *) MIGRATION_TARGET_UNITS="$MIGRATION_TARGET_UNITS $legacy_target_unit" ;;
                esac
            fi
            if run_systemctl --user is-enabled --quiet "$legacy_active_unit" >/dev/null 2>&1; then
                legacy_enable_target=$(canonical_unit_for_legacy "$legacy_active_unit") || return 1
                LEGACY_ENABLED_UNITS="$LEGACY_ENABLED_UNITS $legacy_active_unit"
                case " $CANONICAL_ENABLED_UNITS $MIGRATION_ENABLE_TARGETS " in
                    *" $legacy_enable_target "*) ;;
                    *) MIGRATION_ENABLE_TARGETS="$MIGRATION_ENABLE_TARGETS $legacy_enable_target" ;;
                esac
            fi
        done
    fi
}

unit_was_active() {
    case " $ACTIVE_UNITS " in
        *" $1 "*) return 0 ;;
        *) return 1 ;;
    esac
}

runtime_main_pid() {
    runtime_pid_unit=$1
    runtime_pid=$(run_systemctl --user show "$runtime_pid_unit" \
        --property MainPID --value 2>/dev/null) || return 1
    case "$runtime_pid" in
        ''|*[!0-9]*) return 1 ;;
    esac
    [ "$runtime_pid" -gt 0 ] || return 1
    printf '%s\n' "$runtime_pid"
}

read_process_executable() {
    process_pid=$1
    PROCESS_EXECUTABLE=$(readlink "/proc/$process_pid/exe" 2>/dev/null) || return 1
}

unit_runtime_identity_valid() {
    runtime_identity_unit=$1
    case "$runtime_identity_unit" in
        mitsuro-hive.service) runtime_expected_binary="$INSTALL_DIR/$DAEMON_BINARY" ;;
        mitsuro-serve.service) runtime_expected_binary="$INSTALL_DIR/$BINARY" ;;
        *) return 1 ;;
    esac
    [ -f "$runtime_expected_binary" ] || return 1
    runtime_expected_executable=$(readlink -f "$runtime_expected_binary" 2>/dev/null) || \
        return 1
    [ -f "$runtime_expected_executable" ] && [ ! -L "$runtime_expected_executable" ] || \
        return 1
    runtime_identity_pid=$(runtime_main_pid "$runtime_identity_unit") || return 1
    PROCESS_EXECUTABLE=""
    read_process_executable "$runtime_identity_pid" || return 1
    runtime_observed_executable=$PROCESS_EXECUTABLE
    case "$runtime_observed_executable" in
        *" (deleted)") return 1 ;;
    esac
    [ "$runtime_observed_executable" = "$runtime_expected_executable" ]
}

invoke_hive_ping() {
    hive_runtime_parent=${XDG_RUNTIME_DIR:-}
    if [ -z "$hive_runtime_parent" ]; then
        hive_runtime_uid=$(id -u 2>/dev/null) || return 1
        hive_runtime_parent="/run/user/$hive_runtime_uid"
    fi
    "$INSTALL_DIR/$DAEMON_BINARY" ping \
        --socket "$hive_runtime_parent/mitsuro/hive.sock" \
        --key "$HOME/$CANONICAL_CONFIG_BASENAME/run/hive-ipc.key" \
        >/dev/null 2>&1
}

probe_hive_runtime() {
    invoke_hive_ping || return 1
    run_systemctl --user is-active --quiet mitsuro-hive.service >/dev/null 2>&1 || return 1
    unit_runtime_identity_valid mitsuro-hive.service
}

fetch_server_health() {
    server_health_port=$1
    SERVER_HEALTH_PAYLOAD=$(curl --fail --silent --show-error --max-time 5 \
        "http://127.0.0.1:$server_health_port/health") || return 1
}

probe_server_runtime() {
    server_runtime_pid=$(runtime_main_pid mitsuro-serve.service) || return 1
    server_pid_file="$HOME/$CANONICAL_CONFIG_BASENAME/server.pid"
    [ -f "$server_pid_file" ] && [ ! -L "$server_pid_file" ] || return 1
    [ "$(wc -l < "$server_pid_file" | tr -d '[:space:]')" = 0 ] || \
        [ "$(wc -l < "$server_pid_file" | tr -d '[:space:]')" = 1 ] || return 1
    server_instance_record=$(sed -n '1p' "$server_pid_file") || return 1
    case "$server_instance_record" in
        *:*) ;;
        *) return 1 ;;
    esac
    server_instance_pid=${server_instance_record%%:*}
    server_instance_port=${server_instance_record#*:}
    case "$server_instance_pid:$server_instance_port" in
        *[!0-9:]*|:*|*:) return 1 ;;
    esac
    case "$server_instance_port" in
        *:*) return 1 ;;
    esac
    [ "$server_instance_pid" = "$server_runtime_pid" ] || return 1
    [ "$server_instance_port" -gt 0 ] && [ "$server_instance_port" -le 65535 ] || return 1
    SERVER_HEALTH_PAYLOAD=""
    fetch_server_health "$server_instance_port" 2>/dev/null || return 1
    server_health_payload=$SERVER_HEALTH_PAYLOAD
    printf '%s\n' "$server_health_payload" | \
        grep -Eq '"status"[[:space:]]*:[[:space:]]*"ok"'
}

legacy_unit_runtime_identity_valid() {
    legacy_runtime_identity_unit=$1
    case "$legacy_runtime_identity_unit" in
        "$LEGACY_HIVE_SERVICE_UNIT")
            legacy_runtime_expected_binary="$INSTALL_DIR/$LEGACY_DAEMON_BINARY"
            ;;
        "$LEGACY_SERVE_UNIT")
            legacy_runtime_expected_binary="$INSTALL_DIR/$LEGACY_BINARY"
            ;;
        *) return 1 ;;
    esac
    [ -f "$legacy_runtime_expected_binary" ] || return 1
    legacy_runtime_expected_executable=$(readlink -f \
        "$legacy_runtime_expected_binary" 2>/dev/null) || return 1
    [ -f "$legacy_runtime_expected_executable" ] && \
        [ ! -L "$legacy_runtime_expected_executable" ] || return 1
    legacy_runtime_identity_pid=$(runtime_main_pid "$legacy_runtime_identity_unit") || return 1
    PROCESS_EXECUTABLE=""
    read_process_executable "$legacy_runtime_identity_pid" || return 1
    case "$PROCESS_EXECUTABLE" in
        *" (deleted)") return 1 ;;
    esac
    [ "$PROCESS_EXECUTABLE" = "$legacy_runtime_expected_executable" ]
}

invoke_legacy_hive_ping() {
    legacy_hive_runtime_parent=${XDG_RUNTIME_DIR:-}
    if [ -z "$legacy_hive_runtime_parent" ]; then
        legacy_hive_runtime_uid=$(id -u 2>/dev/null) || return 1
        legacy_hive_runtime_parent="/run/user/$legacy_hive_runtime_uid"
    fi
    "$INSTALL_DIR/$LEGACY_DAEMON_BINARY" ping \
        --socket "$legacy_hive_runtime_parent/$LEGACY_RUNTIME_BASENAME/$LEGACY_SOCKET_BASENAME" \
        --key "$HOME/$LEGACY_CONFIG_BASENAME/run/$LEGACY_HIVE_KEY_BASENAME" \
        >/dev/null 2>&1
}

probe_legacy_hive_runtime() {
    invoke_legacy_hive_ping || return 1
    run_systemctl --user is-active --quiet "$LEGACY_HIVE_SERVICE_UNIT" \
        >/dev/null 2>&1 || return 1
    legacy_unit_runtime_identity_valid "$LEGACY_HIVE_SERVICE_UNIT"
}

probe_legacy_server_runtime() {
    legacy_server_runtime_pid=$(runtime_main_pid "$LEGACY_SERVE_UNIT") || return 1
    legacy_server_pid_file="$HOME/$LEGACY_CONFIG_BASENAME/server.pid"
    [ -f "$legacy_server_pid_file" ] && [ ! -L "$legacy_server_pid_file" ] || return 1
    legacy_server_record=$(sed -n '1p' "$legacy_server_pid_file") || return 1
    case "$legacy_server_record" in
        *:*) ;;
        *) return 1 ;;
    esac
    legacy_server_pid=${legacy_server_record%%:*}
    legacy_server_port=${legacy_server_record#*:}
    case "$legacy_server_pid:$legacy_server_port" in
        *[!0-9:]*|:*|*:) return 1 ;;
    esac
    case "$legacy_server_port" in
        *:*) return 1 ;;
    esac
    [ "$legacy_server_pid" = "$legacy_server_runtime_pid" ] || return 1
    [ "$legacy_server_port" -gt 0 ] && [ "$legacy_server_port" -le 65535 ] || return 1
    SERVER_HEALTH_PAYLOAD=""
    fetch_server_health "$legacy_server_port" 2>/dev/null || return 1
    printf '%s\n' "$SERVER_HEALTH_PAYLOAD" | \
        grep -Eq '"status"[[:space:]]*:[[:space:]]*"ok"'
}

validate_legacy_runtime_authority() {
    legacy_runtime_needs_hive=false
    legacy_runtime_needs_server=false
    for legacy_runtime_unit in $LEGACY_ACTIVE_UNITS; do
        case "$legacy_runtime_unit" in
            "$LEGACY_HIVE_SOCKET_UNIT"|"$LEGACY_HIVE_SERVICE_UNIT")
                legacy_runtime_needs_hive=true
                ;;
            "$LEGACY_SERVE_UNIT")
                legacy_runtime_needs_hive=true
                legacy_runtime_needs_server=true
                ;;
        esac
    done
    if [ "$legacy_runtime_needs_hive" = true ]; then
        probe_legacy_hive_runtime || return 1
    fi
    if [ "$legacy_runtime_needs_server" = true ]; then
        legacy_unit_runtime_identity_valid "$LEGACY_SERVE_UNIT" || return 1
        probe_legacy_server_runtime || return 1
    fi
}

validate_runtime_authority_for_units() {
    runtime_authority_units=$1
    runtime_needs_hive=false
    runtime_needs_server=false
    for runtime_authority_unit in $runtime_authority_units; do
        case "$runtime_authority_unit" in
            mitsuro-hive.socket|mitsuro-hive.service) runtime_needs_hive=true ;;
            mitsuro-serve.service)
                runtime_needs_hive=true
                runtime_needs_server=true
                ;;
        esac
    done
    if [ "$runtime_needs_hive" = true ]; then
        probe_hive_runtime || return 1
    fi
    if [ "$runtime_needs_server" = true ]; then
        unit_runtime_identity_valid mitsuro-serve.service || return 1
        probe_server_runtime || return 1
    fi
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
        if [ "$all_units_active" = true ] && \
            ! validate_runtime_authority_for_units "$ACTIVE_UNITS"; then
            all_units_active=false
        fi
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

stop_legacy_services_for_migration() {
    [ -n "$LEGACY_ACTIVE_UNITS" ] || return 0
    # LEGACY_ACTIVE_UNITS is assembled only from LEGACY_SYSTEMD_UNITS.
    # shellcheck disable=SC2086
    run_systemctl --user stop $LEGACY_ACTIVE_UNITS
}

restart_and_verify_migration_targets() {
    [ -n "$MIGRATION_TARGET_UNITS" ] || return 0
    # MIGRATION_TARGET_UNITS is assembled only from SYSTEMD_UNITS.
    # shellcheck disable=SC2086
    run_systemctl --user restart $MIGRATION_TARGET_UNITS || return 1
    migration_attempt=1
    migration_stable_passes=0
    while [ "$migration_attempt" -le "$ACTIVATION_HEALTH_ATTEMPTS" ]; do
        health_pause
        migration_units_active=true
        for migration_unit in $MIGRATION_TARGET_UNITS; do
            if ! run_systemctl --user is-active --quiet "$migration_unit" >/dev/null 2>&1; then
                migration_units_active=false
                break
            fi
        done
        if [ "$migration_units_active" = true ] && \
            ! validate_runtime_authority_for_units "$MIGRATION_TARGET_UNITS"; then
            migration_units_active=false
        fi
        if [ "$migration_units_active" = true ]; then
            migration_stable_passes=$((migration_stable_passes + 1))
            if [ "$migration_stable_passes" -ge "$ACTIVATION_STABLE_PASSES" ]; then
                return 0
            fi
        else
            migration_stable_passes=0
        fi
        migration_attempt=$((migration_attempt + 1))
    done
    return 1
}

restore_legacy_services_after_rollback() {
    [ -n "$LEGACY_ACTIVE_UNITS" ] || return 0
    # LEGACY_ACTIVE_UNITS is assembled only from LEGACY_SYSTEMD_UNITS.
    # shellcheck disable=SC2086
    run_systemctl --user restart $LEGACY_ACTIVE_UNITS || return 1
    legacy_restore_attempt=1
    legacy_restore_stable_passes=0
    while [ "$legacy_restore_attempt" -le "$ACTIVATION_HEALTH_ATTEMPTS" ]; do
        health_pause
        legacy_restore_active=true
        for restored_legacy_unit in $LEGACY_ACTIVE_UNITS; do
            if ! run_systemctl --user is-active --quiet "$restored_legacy_unit" \
                >/dev/null 2>&1; then
                legacy_restore_active=false
                break
            fi
        done
        if [ "$legacy_restore_active" = true ] && \
            validate_legacy_runtime_authority; then
            legacy_restore_stable_passes=$((legacy_restore_stable_passes + 1))
            if [ "$legacy_restore_stable_passes" -ge "$ACTIVATION_STABLE_PASSES" ]; then
                return 0
            fi
        else
            legacy_restore_stable_passes=0
        fi
        legacy_restore_attempt=$((legacy_restore_attempt + 1))
    done
    return 1
}

migrate_service_enablement() {
    for migration_enable_target in $MIGRATION_ENABLE_TARGETS; do
        case " $CANONICAL_ENABLED_UNITS " in
            *" $migration_enable_target "*) ;;
            *)
                run_systemctl --user enable "$migration_enable_target" || return 1
                NEWLY_ENABLED_CANONICAL_UNITS="$NEWLY_ENABLED_CANONICAL_UNITS $migration_enable_target"
                ;;
        esac
    done
    for legacy_enabled_unit in $LEGACY_ENABLED_UNITS; do
        run_systemctl --user disable "$legacy_enabled_unit" || return 1
        DISABLED_LEGACY_UNITS="$DISABLED_LEGACY_UNITS $legacy_enabled_unit"
    done
}

restore_service_enablement_after_rollback() {
    for newly_enabled_unit in $NEWLY_ENABLED_CANONICAL_UNITS; do
        run_systemctl --user disable "$newly_enabled_unit" || return 1
    done
    for legacy_enabled_unit in $DISABLED_LEGACY_UNITS; do
        run_systemctl --user enable "$legacy_enabled_unit" || return 1
    done
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
    activation_snapshot_restored=true
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ] && systemctl_available; then
        if ! stop_newly_active_candidate_units; then
            echo "Warning: candidate-only services could not all be stopped." >&2
            rollback_failed=true
        fi
    fi
    if ! restore_activation_snapshot; then
        echo "Warning: one or more managed paths could not be restored." >&2
        rollback_failed=true
        activation_snapshot_restored=false
    fi
    state_authority_restored=true
    if ! restore_state_checkpoint; then
        echo "Warning: previous state authority could not be restored; no scheduler will be restarted." >&2
        rollback_failed=true
        state_authority_restored=false
    fi
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_USER_DIR_WAS_PRESENT" != true ]; then
        rmdir "$SYSTEMD_USER_DIR" 2>/dev/null || true
    fi
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ] && \
        systemctl_available; then
        if ! restore_service_enablement_after_rollback; then
            echo "Warning: service enablement could not be restored exactly." >&2
            rollback_failed=true
        fi
    fi
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ] && \
        [ "$state_authority_restored" = true ] && systemctl_available; then
        if ! run_systemctl --user daemon-reload; then
            echo "Warning: systemd could not reload the restored unit set." >&2
            rollback_failed=true
        fi
        if ! restart_and_verify_previously_active; then
            echo "Warning: one or more previously active services could not be restored healthy." >&2
            rollback_failed=true
        fi
        if ! restore_legacy_services_after_rollback; then
            echo "Warning: one or more previous-generation services could not be restarted." >&2
            rollback_failed=true
        fi
    fi
    if [ "$activation_snapshot_restored" = true ]; then
        cleanup_activation_backup
    else
        preserved_activation_backup=$ACTIVATION_BACKUP
        ACTIVATION_BACKUP=""
        echo "Preserved activation recovery files at $preserved_activation_backup" >&2
    fi
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
    CURRENT_LINK="$INSTALL_DIR/.mitsuro-current"
    SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
    SYSTEMD_MARKER="$INSTALL_DIR/.mitsuro-systemd-managed"
    MIGRATING_LEGACY=false
    ADOPTING_CANONICAL_SYSTEMD_UNITS=false
    LEGACY_HAS_SYSTEMD_DIR=false
    SYSTEMD_TRANSITION_STARTED=false

    detect_legacy_installation
    preflight_identity_state_roots || return 1
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
    if [ "$LEGACY_INSTALL_FOUND" = true ]; then
        for legacy_supervised_path in \
            "$INSTALL_DIR/$LEGACY_DAEMON_BINARY" \
            "$INSTALL_DIR/$LEGACY_MARKER_BASENAME"; do
            if [ -e "$legacy_supervised_path" ] || [ -L "$legacy_supervised_path" ]; then
                EXISTING_SUPERVISED_SET=true
            fi
        done
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
        for legacy_existing_unit in $LEGACY_SYSTEMD_UNITS; do
            if [ -e "$SYSTEMD_USER_DIR/$legacy_existing_unit" ] || \
                [ -L "$SYSTEMD_USER_DIR/$legacy_existing_unit" ]; then
                EXISTING_SUPERVISED_SET=true
            fi
        done
    fi
    if [ "$EXISTING_SUPERVISED_SET" = true ] && [ ! -f "$RELEASE_DIR/$DAEMON_BINARY" ]; then
        fail "Refusing to replace a supervised Hive release with a mitsuro-only archive."
        return 1
    fi

    MANAGE_SYSTEMD=false
    if [ "$OS" = "Linux" ] && [ "$INSTALL_DIR" = "$DEFAULT_INSTALL_DIR" ]; then
        if [ -d "$RELEASE_DIR/systemd" ] || [ -f "$SYSTEMD_MARKER" ] || \
            { [ -n "$PREVIOUS_TARGET" ] && [ -d "$CURRENT_LINK/systemd" ]; } || \
            [ "$LEGACY_INSTALL_FOUND" = true ]; then
            MANAGE_SYSTEMD=true
        fi
    elif [ "$OS" = "Linux" ] && [ -d "$RELEASE_DIR/systemd" ]; then
        echo "Skipping systemd units: shipped units target $DEFAULT_INSTALL_DIR/.mitsuro-current exactly."
        echo "Install and override the units explicitly if supervision is required."
    fi

    capture_active_services || return 1
    if [ "$STATE_MIGRATION_PENDING" = true ] && [ -n "$ACTIVE_UNITS" ]; then
        fail "Canonical managed services are already active before the offline identity migration. Stop every Mitsuro CLI, TUI, desktop, server, and Hive process, then retry."
        return 1
    fi
    if [ "$STATE_MIGRATION_RECEIPTED" = true ]; then
        legacy_runtime_parent=${XDG_RUNTIME_DIR:-}
        if [ -z "$legacy_runtime_parent" ] && command -v id >/dev/null 2>&1; then
            legacy_runtime_parent="/run/user/$(id -u)"
        fi
        legacy_runtime_socket="$legacy_runtime_parent/$LEGACY_RUNTIME_BASENAME/$LEGACY_SOCKET_BASENAME"
        if [ -n "$LEGACY_ACTIVE_UNITS" ] || \
            { [ -n "$legacy_runtime_parent" ] && \
              { [ -e "$legacy_runtime_socket" ] || [ -L "$legacy_runtime_socket" ]; }; }; then
            fail "A previous-generation Mitsuro process authority is still live beside receipted canonical state."
            return 1
        fi
    fi
    if ! prepare_activation_snapshot; then
        cleanup_activation_backup
        return 1
    fi
    ACTIVATION_IN_PROGRESS=true

    if [ -z "$PREVIOUS_TARGET" ]; then
        MIGRATING_LEGACY=true
        create_legacy_release "$MANAGE_SYSTEMD" || { fail_activation "legacy release capture failed"; return 1; }
    elif [ "$MANAGE_SYSTEMD" = true ] && \
        [ ! -e "$SYSTEMD_MARKER" ] && [ ! -L "$SYSTEMD_MARKER" ] && \
        canonical_regular_systemd_unit_present; then
        if ! unmarked_canonical_units_adoptable; then
            fail_activation "unmarked canonical systemd unit ownership could not be proven"
            return 1
        fi
        ADOPTING_CANONICAL_SYSTEMD_UNITS=true
    fi

    install_managed_link ".mitsuro-current/$BINARY" "$INSTALL_DIR/$BINARY" "$MIGRATING_LEGACY" || \
        { fail_activation "$BINARY link publication failed"; return 1; }
    activation_checkpoint after-mitsuro-link || { fail_activation "fixture after $BINARY link"; return 1; }
    if [ -f "$RELEASE_DIR/$DAEMON_BINARY" ] || [ -f "$CURRENT_LINK/$DAEMON_BINARY" ] || \
        [ -e "$INSTALL_DIR/$DAEMON_BINARY" ] || [ -L "$INSTALL_DIR/$DAEMON_BINARY" ]; then
        install_managed_link ".mitsuro-current/$DAEMON_BINARY" "$INSTALL_DIR/$DAEMON_BINARY" "$MIGRATING_LEGACY" || \
            { fail_activation "$DAEMON_BINARY link publication failed"; return 1; }
        activation_checkpoint after-hive-link || { fail_activation "fixture after $DAEMON_BINARY link"; return 1; }
    fi

    install_managed_link ".mitsuro-current/$COMPAT_BINARY" \
        "$INSTALL_DIR/$COMPAT_BINARY" "$MIGRATING_LEGACY" \
        "$LEGACY_CURRENT_BASENAME/$COMPAT_BINARY" \
        "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/$COMPAT_BINARY" || \
        { fail_activation "$COMPAT_BINARY compatibility link publication failed"; return 1; }
    activation_checkpoint after-compat-cli-link || \
        { fail_activation "fixture after compatibility CLI link"; return 1; }
    if [ -f "$RELEASE_DIR/agent-browser" ] || [ -f "$CURRENT_LINK/agent-browser" ]; then
        install_managed_link ".mitsuro-current/agent-browser" \
            "$INSTALL_DIR/agent-browser" "$MIGRATING_LEGACY" || \
            { fail_activation "agent-browser link publication failed"; return 1; }
        activation_checkpoint after-atlas-link || \
            { fail_activation "fixture after agent-browser link"; return 1; }
    fi
    if [ -f "$RELEASE_DIR/$COMPAT_DAEMON_BINARY" ] || \
        [ -f "$CURRENT_LINK/$COMPAT_DAEMON_BINARY" ] || \
        [ -e "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" ] || \
        [ -L "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" ]; then
        install_managed_link ".mitsuro-current/$COMPAT_DAEMON_BINARY" \
            "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" "$MIGRATING_LEGACY" \
            "$LEGACY_CURRENT_BASENAME/$COMPAT_DAEMON_BINARY" \
            "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/$COMPAT_DAEMON_BINARY" || \
            { fail_activation "$COMPAT_DAEMON_BINARY compatibility link publication failed"; return 1; }
        activation_checkpoint after-compat-hive-link || \
            { fail_activation "fixture after compatibility Hive link"; return 1; }
    fi

    if [ "$MANAGE_SYSTEMD" = true ]; then
        mkdir -p "$SYSTEMD_USER_DIR" || { fail_activation "systemd user directory creation failed"; return 1; }
        for managed_unit in $SYSTEMD_UNITS; do
            previous_unit_target=""
            previous_unit_target=$(previous_managed_unit_target "$managed_unit") || previous_unit_target=""
            allow_regular_managed_unit=$MIGRATING_LEGACY
            if [ "$ADOPTING_CANONICAL_SYSTEMD_UNITS" = true ]; then
                [ ! -e "$SYSTEMD_MARKER" ] && [ ! -L "$SYSTEMD_MARKER" ] || \
                    { fail_activation "systemd marker changed during canonical unit adoption"; return 1; }
                canonical_unit_unchanged_since_snapshot "$managed_unit" || \
                    { fail_activation "$managed_unit changed during canonical unit adoption"; return 1; }
                allow_regular_managed_unit=true
            fi
            install_managed_link "$CURRENT_LINK/systemd/$managed_unit" "$SYSTEMD_USER_DIR/$managed_unit" \
                "$allow_regular_managed_unit" "$previous_unit_target" || \
                { fail_activation "$managed_unit link publication failed"; return 1; }
            activation_checkpoint "after-$managed_unit-link" || \
                { fail_activation "fixture after $managed_unit link"; return 1; }
        done
        activation_checkpoint before-marker || { fail_activation "fixture before marker publication"; return 1; }
        publish_systemd_marker || { fail_activation "systemd marker publication failed"; return 1; }
        activation_checkpoint after-marker || { fail_activation "fixture after marker publication"; return 1; }
    fi

    if ! atomic_symlink ".mitsuro-releases/$RELEASE_ID" "$CURRENT_LINK"; then
        fail_activation "could not activate release $RELEASE_ID"
        return 1
    fi
    activation_checkpoint after-pointer || { fail_activation "fixture after release pointer"; return 1; }

    if [ "$MANAGE_SYSTEMD" = true ] && systemctl_available; then
        SYSTEMD_TRANSITION_STARTED=true
        if ! stop_legacy_services_for_migration; then
            fail_activation "previous service generation could not be quiesced"
            return 1
        fi
        if ! prove_legacy_state_quiescent "after stopping managed services"; then
            fail_activation "another Mitsuro process still owns previous state"
            return 1
        fi
    fi

    prepare_state_migration_manifest || \
        { fail_activation "state migration preflight failed"; return 1; }
    invoke_canonical_state_migration || \
        { fail_activation "canonical state migration failed"; return 1; }

    if [ "$MANAGE_SYSTEMD" = true ]; then
        if systemctl_available; then
            if ! run_systemctl --user daemon-reload; then
                fail_activation "systemd daemon-reload failed"
                return 1
            fi
            if ! restart_and_verify_previously_active; then
                fail_activation "a previously active service did not settle healthy"
                return 1
            fi
            if ! restart_and_verify_migration_targets; then
                fail_activation "a migrated service did not settle healthy"
                return 1
            fi
        else
            echo "systemctl is unavailable; units were installed but not reloaded."
        fi
    fi
    validate_canonical_state_after_start || \
        { fail_activation "canonical post-start state validation failed"; return 1; }
    if [ "$MANAGE_SYSTEMD" = true ] && [ "$SYSTEMD_TRANSITION_STARTED" = true ]; then
        migrate_service_enablement || \
            { fail_activation "service enablement migration failed"; return 1; }
    fi

    ACTIVATION_IN_PROGRESS=false
    cleanup_activation_backup
    SYSTEMD_TRANSITION_STARTED=false
    INSTALLED_SYSTEMD_UNITS="$MANAGE_SYSTEMD"
}

detect_windows_state_cutover_requirement() {
    WINDOWS_STATE_MIGRATION_REQUIRED=false
    windows_legacy_root="$HOME/$LEGACY_CONFIG_BASENAME"
    if [ -e "$windows_legacy_root" ] || [ -L "$windows_legacy_root" ]; then
        WINDOWS_STATE_MIGRATION_REQUIRED=true
    fi
}

cleanup_windows_stages() {
    if [ "$WINDOWS_STAGES_OWNED" = true ] && [ -n "$WINDOWS_CANONICAL_STAGE" ]; then
        rm -f "$WINDOWS_CANONICAL_STAGE" 2>/dev/null || true
    fi
    if [ "$WINDOWS_STAGES_OWNED" = true ] && [ -n "$WINDOWS_COMPAT_STAGE" ]; then
        rm -f "$WINDOWS_COMPAT_STAGE" 2>/dev/null || true
    fi
    WINDOWS_CANONICAL_STAGE=""
    WINDOWS_COMPAT_STAGE=""
    WINDOWS_STAGES_OWNED=false
}

cleanup_windows_publication_backup() {
    remove_writable_tree "$WINDOWS_PUBLICATION_BACKUP"
    WINDOWS_PUBLICATION_BACKUP=""
}

snapshot_windows_destination() {
    windows_snapshot_key=$1
    windows_snapshot_path=$2
    windows_snapshot_state="$WINDOWS_PUBLICATION_BACKUP/$windows_snapshot_key.state"
    if [ -f "$windows_snapshot_path" ] && [ ! -L "$windows_snapshot_path" ]; then
        printf '%s\n' file > "$windows_snapshot_state" || return 1
        cp -p "$windows_snapshot_path" \
            "$WINDOWS_PUBLICATION_BACKUP/$windows_snapshot_key.file" || return 1
    elif [ -e "$windows_snapshot_path" ] || [ -L "$windows_snapshot_path" ]; then
        fail "Refusing non-regular Windows destination $windows_snapshot_path."
        return 1
    else
        printf '%s\n' absent > "$windows_snapshot_state" || return 1
    fi
}

restore_windows_destination() {
    windows_restore_key=$1
    windows_restore_path=$2
    if [ "$SELF_TEST_FAIL_WINDOWS_RESTORE" = true ] && \
        [ "$windows_restore_key" = canonical ]; then
        SELF_TEST_FAIL_WINDOWS_RESTORE=false
        return 1
    fi
    windows_restore_state="$WINDOWS_PUBLICATION_BACKUP/$windows_restore_key.state"
    windows_restore_kind=$(sed -n '1p' "$windows_restore_state") || return 1
    case "$windows_restore_kind" in
        absent)
            if [ -d "$windows_restore_path" ] && [ ! -L "$windows_restore_path" ]; then
                return 1
            fi
            rm -f "$windows_restore_path"
            ;;
        file)
            windows_restore_parent=$(dirname "$windows_restore_path")
            windows_restore_leaf=$(basename "$windows_restore_path")
            windows_restore_tmp="$windows_restore_parent/.$windows_restore_leaf.mitsuro-restore.$$"
            rm -f "$windows_restore_tmp"
            ATOMIC_TEMP="$windows_restore_tmp"
            cp -p "$WINDOWS_PUBLICATION_BACKUP/$windows_restore_key.file" \
                "$windows_restore_tmp" || { cleanup_atomic_temp; return 1; }
            atomic_replace_path "$windows_restore_tmp" "$windows_restore_path" || \
                { cleanup_atomic_temp; return 1; }
            ATOMIC_TEMP=""
            ;;
        *)
            fail "Invalid Windows publication snapshot for $windows_restore_path."
            return 1
            ;;
    esac
}

prepare_windows_publication_backup() {
    WINDOWS_PUBLICATION_BACKUP="$INSTALL_DIR/.mitsuro-windows-backup.$$"
    if [ -e "$WINDOWS_PUBLICATION_BACKUP" ] || [ -L "$WINDOWS_PUBLICATION_BACKUP" ]; then
        fail "Unexpected Windows publication backup already exists: $WINDOWS_PUBLICATION_BACKUP"
        return 1
    fi
    mkdir "$WINDOWS_PUBLICATION_BACKUP" || return 1
    chmod 0700 "$WINDOWS_PUBLICATION_BACKUP" || return 1
    snapshot_windows_destination canonical "$WINDOWS_CANONICAL_DESTINATION" || return 1
    snapshot_windows_destination compatibility "$WINDOWS_COMPAT_DESTINATION" || return 1
}

rollback_windows_publication() {
    windows_restore_failed=false
    windows_preserved_backup=""
    WINDOWS_PUBLICATION_IN_PROGRESS=false
    cleanup_windows_stages
    if [ -n "$WINDOWS_PUBLICATION_BACKUP" ]; then
        restore_windows_destination canonical "$WINDOWS_CANONICAL_DESTINATION" || \
            windows_restore_failed=true
        restore_windows_destination compatibility "$WINDOWS_COMPAT_DESTINATION" || \
            windows_restore_failed=true
    fi
    if [ "$windows_restore_failed" = false ]; then
        cleanup_windows_publication_backup
        WINDOWS_CANONICAL_DESTINATION=""
        WINDOWS_COMPAT_DESTINATION=""
    else
        windows_preserved_backup=$WINDOWS_PUBLICATION_BACKUP
        WINDOWS_PUBLICATION_BACKUP=""
        echo "Preserved Windows command-pair recovery files at $windows_preserved_backup" >&2
    fi
    [ "$windows_restore_failed" = false ]
}

fail_windows_publication() {
    windows_failure_reason=$1
    if ! rollback_windows_publication; then
        echo "Warning: Windows command-pair rollback completed with errors; do not run either command until both files are restored from the preserved recovery directory." >&2
    fi
    fail "$windows_failure_reason"
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
    windows_compat_candidate="$windows_payload/$COMPAT_BINARY.exe"
    if [ -e "$windows_compat_candidate" ] || [ -L "$windows_compat_candidate" ]; then
        if [ ! -f "$windows_compat_candidate" ] || [ -L "$windows_compat_candidate" ]; then
            fail "Refusing unsafe compatibility payload $windows_compat_candidate."
            return 1
        fi
        if ! cmp -s "$windows_source" "$windows_compat_candidate"; then
            fail "The Windows compatibility payload must be byte-identical to $BINARY.exe."
            return 1
        fi
    fi

    # Canonical Windows archives intentionally contain exactly one mitsuro.exe
    # entry. Publish the transition command as a full byte-for-byte copy so it
    # remains self-contained when invoked without a canonical sibling.
    windows_compat_source="$windows_source"

    echo "Installing directly to $INSTALL_DIR..."
    acquire_install_lock || return 1

    WINDOWS_CANONICAL_DESTINATION="$INSTALL_DIR/$BINARY.exe"
    if [ -e "$WINDOWS_CANONICAL_DESTINATION" ] || \
        [ -L "$WINDOWS_CANONICAL_DESTINATION" ]; then
        if [ ! -f "$WINDOWS_CANONICAL_DESTINATION" ] || \
            [ -L "$WINDOWS_CANONICAL_DESTINATION" ]; then
            fail "Refusing to replace non-regular Windows destination $WINDOWS_CANONICAL_DESTINATION."
            return 1
        fi
    fi
    WINDOWS_COMPAT_DESTINATION="$INSTALL_DIR/$COMPAT_BINARY.exe"
    if [ -e "$WINDOWS_COMPAT_DESTINATION" ] || \
        [ -L "$WINDOWS_COMPAT_DESTINATION" ]; then
        if [ ! -f "$WINDOWS_COMPAT_DESTINATION" ] || \
            [ -L "$WINDOWS_COMPAT_DESTINATION" ]; then
            fail "Refusing to replace non-regular Windows destination $WINDOWS_COMPAT_DESTINATION."
            return 1
        fi
    fi

    WINDOWS_CANONICAL_STAGE="$INSTALL_DIR/.$BINARY.exe.mitsuro-new.$$"
    WINDOWS_COMPAT_STAGE="$INSTALL_DIR/.$COMPAT_BINARY.exe.mitsuro-new.$$"
    if [ -e "$WINDOWS_CANONICAL_STAGE" ] || [ -L "$WINDOWS_CANONICAL_STAGE" ]; then
        fail "Unexpected Windows staging path already exists: $WINDOWS_CANONICAL_STAGE"
        return 1
    fi
    if [ -e "$WINDOWS_COMPAT_STAGE" ] || [ -L "$WINDOWS_COMPAT_STAGE" ]; then
        fail "Unexpected Windows compatibility staging path already exists: $WINDOWS_COMPAT_STAGE"
        WINDOWS_CANONICAL_STAGE=""
        WINDOWS_COMPAT_STAGE=""
        return 1
    fi

    WINDOWS_STAGES_OWNED=true
    if ! cp "$windows_source" "$WINDOWS_CANONICAL_STAGE" || \
        ! chmod 0755 "$WINDOWS_CANONICAL_STAGE"; then
        cleanup_windows_stages
        fail "Could not stage $BINARY.exe in $INSTALL_DIR."
        return 1
    fi
    if ! cp "$windows_compat_source" "$WINDOWS_COMPAT_STAGE" || \
        ! chmod 0755 "$WINDOWS_COMPAT_STAGE"; then
        cleanup_windows_stages
        fail "Could not stage $COMPAT_BINARY.exe in $INSTALL_DIR."
        return 1
    fi
    if ! cmp -s "$WINDOWS_CANONICAL_STAGE" "$WINDOWS_COMPAT_STAGE"; then
        cleanup_windows_stages
        fail "Staged Windows command pair is not byte-identical."
        return 1
    fi
    if [ "$SELF_TEST_FAIL_POINT" = windows-before-publish ]; then
        cleanup_windows_stages
        fail "Fixture stopped Windows installation before publication."
        return 1
    fi

    if ! prepare_windows_publication_backup; then
        cleanup_windows_stages
        cleanup_windows_publication_backup
        WINDOWS_CANONICAL_DESTINATION=""
        WINDOWS_COMPAT_DESTINATION=""
        return 1
    fi
    WINDOWS_PUBLICATION_IN_PROGRESS=true

    # Recheck immediately before publication. The install lock excludes other
    # cooperating installers, and this also fails closed if the path changed.
    if [ -e "$WINDOWS_CANONICAL_DESTINATION" ] || \
        [ -L "$WINDOWS_CANONICAL_DESTINATION" ]; then
        if [ ! -f "$WINDOWS_CANONICAL_DESTINATION" ] || \
            [ -L "$WINDOWS_CANONICAL_DESTINATION" ]; then
            fail_windows_publication \
                "Refusing changed Windows destination $WINDOWS_CANONICAL_DESTINATION."
            return 1
        fi
    fi
    if [ -e "$WINDOWS_COMPAT_DESTINATION" ] || \
        [ -L "$WINDOWS_COMPAT_DESTINATION" ]; then
        if [ ! -f "$WINDOWS_COMPAT_DESTINATION" ] || \
            [ -L "$WINDOWS_COMPAT_DESTINATION" ]; then
            fail_windows_publication \
                "Refusing changed Windows compatibility destination $WINDOWS_COMPAT_DESTINATION."
            return 1
        fi
    fi
    if ! atomic_replace_path \
        "$WINDOWS_CANONICAL_STAGE" "$WINDOWS_CANONICAL_DESTINATION"; then
        fail_windows_publication \
            "Could not atomically publish $WINDOWS_CANONICAL_DESTINATION."
        return 1
    fi
    WINDOWS_CANONICAL_STAGE=""
    if [ "$SELF_TEST_FAIL_POINT" = windows-after-canonical-publish ]; then
        fail_windows_publication \
            "Fixture stopped Windows installation after canonical publication."
        return 1
    fi

    if ! atomic_replace_path \
        "$WINDOWS_COMPAT_STAGE" "$WINDOWS_COMPAT_DESTINATION"; then
        fail_windows_publication \
            "Could not atomically publish $WINDOWS_COMPAT_DESTINATION."
        return 1
    fi
    WINDOWS_COMPAT_STAGE=""
    if ! regular_file_with_mode "$WINDOWS_CANONICAL_DESTINATION" 755 || \
        ! regular_file_with_mode "$WINDOWS_COMPAT_DESTINATION" 755 || \
        ! cmp -s "$WINDOWS_CANONICAL_DESTINATION" "$WINDOWS_COMPAT_DESTINATION" || \
        ! cmp -s "$windows_source" "$WINDOWS_CANONICAL_DESTINATION"; then
        fail_windows_publication \
            "Published Windows command pair failed verification."
        return 1
    fi

    WINDOWS_PUBLICATION_IN_PROGRESS=false
    cleanup_windows_publication_backup
    cleanup_windows_stages
    WINDOWS_CANONICAL_DESTINATION=""
    WINDOWS_COMPAT_DESTINATION=""
    detect_windows_state_cutover_requirement
}

run_self_test() (
    set -e
    self_root="$(mktemp -d)"
    self_cleanup() {
        chmod -R u+w "$self_root" 2>/dev/null || true
        rm -rf "$self_root"
    }
    trap self_cleanup 0 HUP INT TERM
    for self_valid_tag in v0.9.23 v1.2.3-rc.1 v1.2.3+build.4; do
        valid_release_tag "$self_valid_tag"
    done
    for self_invalid_tag in v0.9 v1.2.3/../../attacker v1.2.3_bad; do
        if valid_release_tag "$self_invalid_tag"; then
            fail "Self-test accepted invalid release tag: $self_invalid_tag"
            exit 1
        fi
    done
    self_multiline_tag=$(printf 'v1.2.3\nattacker')
    if valid_release_tag "$self_multiline_tag"; then
        fail "Self-test accepted a multiline release tag."
        exit 1
    fi
    HOME="$self_root/home"
    export HOME
    XDG_RUNTIME_DIR="$HOME/runtime"
    export XDG_RUNTIME_DIR
    DEFAULT_INSTALL_DIR="$HOME/.local/bin"
    INSTALL_DIR="$DEFAULT_INSTALL_DIR"
    OS="Linux"
    SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
    RELEASES_DIR="$INSTALL_DIR/.mitsuro-releases"
    mkdir -p "$RELEASES_DIR" "$SYSTEMD_USER_DIR" "$XDG_RUNTIME_DIR"
    printf '#!/bin/sh\nprintf "%%s\\n" "direct"\n' > "$INSTALL_DIR/mitsuro"
    chmod 0755 "$INSTALL_DIR/mitsuro"
    printf '%s\n' '# legacy serve only' > "$SYSTEMD_USER_DIR/mitsuro-serve.service"
    chmod 0644 "$SYSTEMD_USER_DIR/mitsuro-serve.service"

    systemctl_available() { return 0; }
    health_pause() { :; }
    SELF_ACTIVE_UNITS=" mitsuro-serve.service"
    SELF_ENABLED_UNITS=" mitsuro-serve.service"
    SELF_LOG=""
    SELF_AFTER_RESTART=false
    SELF_ADD_DEPENDENCY=false
    SELF_FAIL_RELOAD_ONCE=false
    SELF_FAIL_RESTART_ONCE=false
    SELF_FAIL_HEALTH_COUNT=0
    SELF_HEALTH_CHECKS=0
    SELF_FAIL_RUNTIME_IDENTITY_COUNT=0
    SELF_RUNTIME_IDENTITY_CHECKS=0
    SELF_FAIL_HIVE_PING_COUNT=0
    SELF_HIVE_PING_CHECKS=0
    SELF_FAIL_SERVER_HEALTH_COUNT=0
    SELF_SERVER_HEALTH_CHECKS=0

    self_unit_active() {
        case " $SELF_ACTIVE_UNITS " in
            *" $1 "*) return 0 ;;
            *) return 1 ;;
        esac
    }
    self_add_active() {
        self_unit_active "$1" || SELF_ACTIVE_UNITS="$SELF_ACTIVE_UNITS $1"
    }
    self_unit_enabled() {
        case " $SELF_ENABLED_UNITS " in
            *" $1 "*) return 0 ;;
            *) return 1 ;;
        esac
    }
    self_add_enabled() {
        self_unit_enabled "$1" || SELF_ENABLED_UNITS="$SELF_ENABLED_UNITS $1"
    }
    self_remove_enabled() {
        self_enabled_remaining=""
        for self_enabled in $SELF_ENABLED_UNITS; do
            [ "$self_enabled" = "$1" ] || \
                self_enabled_remaining="$self_enabled_remaining $self_enabled"
        done
        SELF_ENABLED_UNITS="$self_enabled_remaining"
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
                if [ "$SELF_AFTER_RESTART" = true ] && [ "$1" = "mitsuro-serve.service" ]; then
                    SELF_HEALTH_CHECKS=$((SELF_HEALTH_CHECKS + 1))
                    if [ "$SELF_FAIL_HEALTH_COUNT" -gt 0 ]; then
                        SELF_FAIL_HEALTH_COUNT=$((SELF_FAIL_HEALTH_COUNT - 1))
                        return 1
                    fi
                fi
                self_unit_active "$1"
                ;;
            is-enabled)
                [ "$1" = "--quiet" ] || return 94
                shift
                self_unit_enabled "$1"
                ;;
            show)
                [ "$#" -eq 4 ] || return 97
                self_show_unit=$1
                [ "$2" = "--property" ] && [ "$3" = MainPID ] && \
                    [ "$4" = "--value" ] || return 97
                self_unit_active "$self_show_unit" || return 1
                case "$self_show_unit" in
                    mitsuro-hive.service) printf '%s\n' 4101 ;;
                    mitsuro-serve.service) printf '%s\n' 4102 ;;
                    "$LEGACY_HIVE_SERVICE_UNIT") printf '%s\n' 4201 ;;
                    "$LEGACY_SERVE_UNIT") printf '%s\n' 4202 ;;
                    *) return 97 ;;
                esac
                ;;
            daemon-reload)
                SELF_LOG="$SELF_LOG|daemon-reload"
                if [ "$SELF_FAIL_RELOAD_ONCE" = true ]; then
                    SELF_FAIL_RELOAD_ONCE=false
                    return 1
                fi
                ;;
            restart)
                [ "$#" -eq 1 ] || return 92
                case "$1" in
                    mitsuro-hive.socket|mitsuro-hive.service|mitsuro-serve.service|\
                    "$LEGACY_HIVE_SOCKET_UNIT"|"$LEGACY_HIVE_SERVICE_UNIT"|\
                    "$LEGACY_SERVE_UNIT") ;;
                    *) return 92 ;;
                esac
                SELF_LOG="$SELF_LOG|restart:$1"
                self_add_active "$1"
                if [ "$1" = "mitsuro-serve.service" ]; then
                    SELF_AFTER_RESTART=true
                    mkdir -p "$HOME/$CANONICAL_CONFIG_BASENAME"
                    printf '%s\n' '4102:3000' > \
                        "$HOME/$CANONICAL_CONFIG_BASENAME/server.pid"
                fi
                if [ "$1" = "$LEGACY_SERVE_UNIT" ]; then
                    mkdir -p "$HOME/$LEGACY_CONFIG_BASENAME"
                    printf '%s\n' '4202:3000' > \
                        "$HOME/$LEGACY_CONFIG_BASENAME/server.pid"
                fi
                [ "$SELF_ADD_DEPENDENCY" = true ] && self_add_active mitsuro-hive.socket
                if [ "$SELF_FAIL_RESTART_ONCE" = true ]; then
                    SELF_FAIL_RESTART_ONCE=false
                    return 1
                fi
                ;;
            stop)
                SELF_LOG="$SELF_LOG|stop:$*"
                for self_stopped in "$@"; do self_remove_active "$self_stopped"; done
                ;;
            enable)
                [ "$#" -eq 1 ] || return 95
                SELF_LOG="$SELF_LOG|enable:$1"
                self_add_enabled "$1"
                ;;
            disable)
                [ "$#" -eq 1 ] || return 96
                SELF_LOG="$SELF_LOG|disable:$1"
                self_remove_enabled "$1"
                ;;
            *) return 93 ;;
        esac
    }

    read_process_executable() {
        self_runtime_pid=$1
        SELF_RUNTIME_IDENTITY_CHECKS=$((SELF_RUNTIME_IDENTITY_CHECKS + 1))
        if [ "$SELF_FAIL_RUNTIME_IDENTITY_COUNT" -gt 0 ]; then
            SELF_FAIL_RUNTIME_IDENTITY_COUNT=$((SELF_FAIL_RUNTIME_IDENTITY_COUNT - 1))
            PROCESS_EXECUTABLE="$self_root/unrelated-runtime"
            return 0
        fi
        case "$self_runtime_pid" in
            4101) PROCESS_EXECUTABLE=$(readlink -f "$INSTALL_DIR/$DAEMON_BINARY") ;;
            4102) PROCESS_EXECUTABLE=$(readlink -f "$INSTALL_DIR/$BINARY") ;;
            4201) PROCESS_EXECUTABLE=$(readlink -f "$INSTALL_DIR/$LEGACY_DAEMON_BINARY") ;;
            4202) PROCESS_EXECUTABLE=$(readlink -f "$INSTALL_DIR/$LEGACY_BINARY") ;;
            *) return 1 ;;
        esac
    }
    invoke_hive_ping() {
        SELF_HIVE_PING_CHECKS=$((SELF_HIVE_PING_CHECKS + 1))
        if [ "$SELF_FAIL_HIVE_PING_COUNT" -gt 0 ]; then
            SELF_FAIL_HIVE_PING_COUNT=$((SELF_FAIL_HIVE_PING_COUNT - 1))
            return 1
        fi
        self_add_active mitsuro-hive.service
    }
    invoke_legacy_hive_ping() {
        SELF_HIVE_PING_CHECKS=$((SELF_HIVE_PING_CHECKS + 1))
        if [ "$SELF_FAIL_HIVE_PING_COUNT" -gt 0 ]; then
            SELF_FAIL_HIVE_PING_COUNT=$((SELF_FAIL_HIVE_PING_COUNT - 1))
            return 1
        fi
        self_add_active "$LEGACY_HIVE_SERVICE_UNIT"
    }
    fetch_server_health() {
        [ "$1" = 3000 ] || return 1
        SELF_SERVER_HEALTH_CHECKS=$((SELF_SERVER_HEALTH_CHECKS + 1))
        if [ "$SELF_FAIL_SERVER_HEALTH_COUNT" -gt 0 ]; then
            SELF_FAIL_SERVER_HEALTH_COUNT=$((SELF_FAIL_SERVER_HEALTH_COUNT - 1))
            return 1
        fi
        SERVER_HEALTH_PAYLOAD='{"status":"ok"}'
    }

    write_self_test_payload() {
        self_payload=$1
        self_value=$2
        self_kind=$3
        mkdir -p "$self_payload"
        printf '#!/bin/sh\nif [ -n "${MITSURO_INSTALL_TEST_SOURCE_ROOT:-}" ]; then\n  cp -a "$MITSURO_INSTALL_TEST_SOURCE_ROOT" "$MITSURO_INSTALL_TEST_TARGET_ROOT"\n  if [ -e "$MITSURO_INSTALL_TEST_TARGET_ROOT/$MITSURO_INSTALL_TEST_SOURCE_DB" ]; then\n    mv "$MITSURO_INSTALL_TEST_TARGET_ROOT/$MITSURO_INSTALL_TEST_SOURCE_DB" "$MITSURO_INSTALL_TEST_TARGET_ROOT/$MITSURO_INSTALL_TEST_TARGET_DB"\n  fi\n  created_unix=$(date +%%s)\n  receipt_sha=0000000000000000000000000000000000000000000000000000000000000000\n  printf "version=2\\nsource=%%s\\ncreated_unix=%%s\\nrollback_preserved=true\\nsource_authority_fingerprint=sqlite=%%s;main_len=1;main_mtime_ns=1;wal_len=absent;wal_mtime_ns=absent|tree_sha256=%%s|tree_stat_sha256=%%s\\n" "$MITSURO_INSTALL_TEST_SOURCE_ROOT" "$created_unix" "$receipt_sha" "$receipt_sha" "$receipt_sha" > "$MITSURO_INSTALL_TEST_TARGET_ROOT/.identity-migration-v2"\nfi\nprintf "%%s\\n" "%s"\n' \
            "$self_value" > "$self_payload/$BINARY"
        chmod 0755 "$self_payload/$BINARY"
        printf '#!/bin/sh\nself_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$self_dir/%s" "$@"\n' \
            "$BINARY" > "$self_payload/$COMPAT_BINARY"
        chmod 0755 "$self_payload/$COMPAT_BINARY"
        if [ "$self_kind" = complete ]; then
            mkdir "$self_payload/systemd"
            printf '#!/bin/sh\nprintf "%%s\\n" "atlas-%s"\n' \
                "$self_value" > "$self_payload/agent-browser"
            chmod 0755 "$self_payload/agent-browser"
            printf '#!/bin/sh\nprintf "%%s\\n" "hive-%s"\n' \
                "$self_value" > "$self_payload/$DAEMON_BINARY"
            chmod 0755 "$self_payload/$DAEMON_BINARY"
            printf '#!/bin/sh\nself_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$self_dir/%s" "$@"\n' \
                "$DAEMON_BINARY" > "$self_payload/$COMPAT_DAEMON_BINARY"
            chmod 0755 "$self_payload/$COMPAT_DAEMON_BINARY"
            for self_unit in $SYSTEMD_UNITS; do
                if [ "$self_unit" = mitsuro-serve.service ]; then
                    cat > "$self_payload/systemd/$self_unit" <<EOF
[Unit]
Description=Mitsuro server fixture $self_value

[Service]
Type=simple
WorkingDirectory=%h
ExecStart=%h/.local/bin/.mitsuro-current/mitsuro serve --port 3000
Environment=RUST_LOG=info
Environment=MITSURO_AGENT_BROWSER_PATH=%h/.local/bin/.mitsuro-current/agent-browser
EOF
                else
                    printf '# fixture %s %s\n' "$self_unit" "$self_value" > "$self_payload/systemd/$self_unit"
                fi
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
            "$INSTALL_DIR"/.mitsuro-activation-backup.* \
            "$INSTALL_DIR"/.*.mitsuro-new.* \
            "$INSTALL_DIR"/.*.mitsuro-restore.* \
            "$SYSTEMD_USER_DIR"/.*.mitsuro-new.* \
            "$SYSTEMD_USER_DIR"/.*.mitsuro-restore.*; do
            [ ! -e "$self_residue" ] && [ ! -L "$self_residue" ] || return 1
        done
    }
    assert_direct_baseline() {
        [ ! -e "$INSTALL_DIR/.mitsuro-current" ] && [ ! -L "$INSTALL_DIR/.mitsuro-current" ]
        [ -f "$INSTALL_DIR/mitsuro" ] && [ ! -L "$INSTALL_DIR/mitsuro" ]
        [ "$("$INSTALL_DIR/mitsuro")" = direct ]
        [ ! -e "$INSTALL_DIR/agent-browser" ] && [ ! -L "$INSTALL_DIR/agent-browser" ]
        [ ! -e "$INSTALL_DIR/$COMPAT_BINARY" ] && [ ! -L "$INSTALL_DIR/$COMPAT_BINARY" ]
        [ ! -e "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" ] && \
            [ ! -L "$INSTALL_DIR/$COMPAT_DAEMON_BINARY" ]
        [ ! -e "$INSTALL_DIR/mitsuro-hive" ] && [ ! -L "$INSTALL_DIR/mitsuro-hive" ]
        [ -f "$SYSTEMD_USER_DIR/mitsuro-serve.service" ] && [ ! -L "$SYSTEMD_USER_DIR/mitsuro-serve.service" ]
        grep -Fqx '# legacy serve only' "$SYSTEMD_USER_DIR/mitsuro-serve.service"
        [ ! -e "$SYSTEMD_USER_DIR/mitsuro-hive.socket" ] && [ ! -L "$SYSTEMD_USER_DIR/mitsuro-hive.socket" ]
        [ ! -e "$SYSTEMD_USER_DIR/mitsuro-hive.service" ] && [ ! -L "$SYSTEMD_USER_DIR/mitsuro-hive.service" ]
        [ ! -e "$INSTALL_DIR/.mitsuro-systemd-managed" ] && [ ! -L "$INSTALL_DIR/.mitsuro-systemd-managed" ]
        assert_no_activation_residue
    }
    reset_self_systemd() {
        SELF_ACTIVE_UNITS=" mitsuro-serve.service"
        SELF_ENABLED_UNITS=" mitsuro-serve.service"
        SELF_LOG=""
        SELF_AFTER_RESTART=false
        SELF_ADD_DEPENDENCY=false
        SELF_FAIL_RELOAD_ONCE=false
        SELF_FAIL_RESTART_ONCE=false
        SELF_FAIL_HEALTH_COUNT=0
        SELF_HEALTH_CHECKS=0
        SELF_FAIL_RUNTIME_IDENTITY_COUNT=0
        SELF_RUNTIME_IDENTITY_CHECKS=0
        SELF_FAIL_HIVE_PING_COUNT=0
        SELF_HIVE_PING_CHECKS=0
        SELF_FAIL_SERVER_HEALTH_COUNT=0
        SELF_SERVER_HEALTH_CHECKS=0
    }
    release_self_install_lock() {
        [ "$LOCK_HELD" = true ]
        [ "$INSTALL_LOCK" = "$INSTALL_DIR/.mitsuro-install.lock" ]
        rmdir "$INSTALL_LOCK"
        LOCK_HELD=false
        INSTALL_LOCK=""
    }
    assert_no_windows_stage() {
        [ -z "$ATOMIC_TEMP" ]
        [ -z "$WINDOWS_CANONICAL_STAGE" ]
        [ -z "$WINDOWS_COMPAT_STAGE" ]
        [ "$WINDOWS_STAGES_OWNED" = false ]
        [ "$WINDOWS_PUBLICATION_IN_PROGRESS" = false ]
        [ -z "$WINDOWS_PUBLICATION_BACKUP" ]
        for self_windows_binary in "$BINARY" "$COMPAT_BINARY"; do
            for self_windows_stage in "$INSTALL_DIR"/.$self_windows_binary.exe.mitsuro-new.*; do
                [ ! -e "$self_windows_stage" ] && [ ! -L "$self_windows_stage" ] || return 1
            done
        done
        for self_windows_backup in "$INSTALL_DIR"/.mitsuro-windows-backup.*; do
            [ ! -e "$self_windows_backup" ] && [ ! -L "$self_windows_backup" ] || return 1
        done
    }
    assert_unmanaged_symlink_rollback() {
        self_link_target=$1
        self_link_label=$2
        self_link_path="$INSTALL_DIR/mitsuro-hive"
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

    run_previous_identity_release_fixture() (
        fixture_outcome=$1
        fixture_home="$self_root/previous-identity-$fixture_outcome"
        HOME="$fixture_home"
        export HOME
        XDG_RUNTIME_DIR="$HOME/runtime"
        export XDG_RUNTIME_DIR
        DEFAULT_INSTALL_DIR="$HOME/.local/bin"
        INSTALL_DIR="$DEFAULT_INSTALL_DIR"
        SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
        RELEASES_DIR="$INSTALL_DIR/.mitsuro-releases"
        old_releases_dir="$INSTALL_DIR/$LEGACY_RELEASES_BASENAME"
        old_release_dir="$old_releases_dir/fixture-release"
        mkdir -p \
            "$RELEASES_DIR" \
            "$SYSTEMD_USER_DIR" \
            "$XDG_RUNTIME_DIR" \
            "$old_release_dir/systemd"

        printf '#!/bin/sh\nprintf "%%s\\n" "previous-cli"\n' > \
            "$old_release_dir/$LEGACY_BINARY"
        printf '#!/bin/sh\nprintf "%%s\\n" "previous-hive"\n' > \
            "$old_release_dir/$LEGACY_DAEMON_BINARY"
        chmod 0555 \
            "$old_release_dir/$LEGACY_BINARY" \
            "$old_release_dir/$LEGACY_DAEMON_BINARY"
        for old_fixture_unit in $LEGACY_SYSTEMD_UNITS; do
            printf '# previous fixture %s\n' "$old_fixture_unit" > \
                "$old_release_dir/systemd/$old_fixture_unit"
            chmod 0444 "$old_release_dir/systemd/$old_fixture_unit"
            ln -s \
                "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/systemd/$old_fixture_unit" \
                "$SYSTEMD_USER_DIR/$old_fixture_unit"
        done
        chmod 0555 "$old_release_dir/systemd" "$old_release_dir"

        old_pointer_target="$LEGACY_RELEASES_BASENAME/fixture-release"
        if [ "$fixture_outcome" = failure ]; then
            old_pointer_target="$old_release_dir"
        fi
        ln -s "$old_pointer_target" "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME"
        ln -s "$LEGACY_CURRENT_BASENAME/$LEGACY_BINARY" \
            "$INSTALL_DIR/$LEGACY_BINARY"
        ln -s "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/$LEGACY_DAEMON_BINARY" \
            "$INSTALL_DIR/$LEGACY_DAEMON_BINARY"
        printf '%s\n' previous-marker > "$INSTALL_DIR/$LEGACY_MARKER_BASENAME"
        chmod 0600 "$INSTALL_DIR/$LEGACY_MARKER_BASENAME"

        stage_self_test_release "previous-$fixture_outcome" \
            "previous-$fixture_outcome-candidate" complete \
            5555555555555555555555555555555555555555555555555555555555555555
        candidate_release_id=$RELEASE_ID
        reset_self_systemd
        SELF_ACTIVE_UNITS=" $LEGACY_SERVE_UNIT"
        SELF_ENABLED_UNITS=" $LEGACY_SERVE_UNIT"
        if [ "$fixture_outcome" = failure ]; then
            SELF_FAIL_RELOAD_ONCE=true
            if activate_unix_release; then
                fail "Self-test expected previous-identity activation rollback."
                exit 1
            fi
        else
            activate_unix_release
        fi

        [ -L "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME" ]
        [ "$(readlink "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME")" = \
            "$old_pointer_target" ]
        [ "$("$old_release_dir/$LEGACY_BINARY")" = previous-cli ]
        [ "$("$old_release_dir/$LEGACY_DAEMON_BINARY")" = previous-hive ]
        grep -Fqx previous-marker "$INSTALL_DIR/$LEGACY_MARKER_BASENAME"
        for old_fixture_unit in $LEGACY_SYSTEMD_UNITS; do
            [ -L "$SYSTEMD_USER_DIR/$old_fixture_unit" ]
            [ "$(readlink "$SYSTEMD_USER_DIR/$old_fixture_unit")" = \
                "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/systemd/$old_fixture_unit" ]
        done

        captured_release=""
        captured_count=0
        for captured_candidate in "$RELEASES_DIR"/legacy-*; do
            if [ -d "$captured_candidate" ] && [ ! -L "$captured_candidate" ]; then
                captured_release=$captured_candidate
                captured_count=$((captured_count + 1))
            fi
        done
        [ "$captured_count" -eq 1 ]
        [ "$("$captured_release/$BINARY")" = previous-cli ]
        [ "$("$captured_release/$COMPAT_BINARY")" = previous-cli ]
        [ "$("$captured_release/$DAEMON_BINARY")" = previous-hive ]
        [ "$("$captured_release/$COMPAT_DAEMON_BINARY")" = previous-hive ]
        [ "$(sed -n '1p' "$captured_release/.previous-release-target")" = \
            "$old_pointer_target" ]
        for old_fixture_unit in $LEGACY_SYSTEMD_UNITS; do
            cmp -s \
                "$old_release_dir/systemd/$old_fixture_unit" \
                "$captured_release/previous-systemd/$old_fixture_unit"
        done

        if [ "$fixture_outcome" = failure ]; then
            [ ! -e "$INSTALL_DIR/.mitsuro-current" ] && \
                [ ! -L "$INSTALL_DIR/.mitsuro-current" ]
            [ ! -e "$INSTALL_DIR/$BINARY" ] && [ ! -L "$INSTALL_DIR/$BINARY" ]
            [ ! -e "$INSTALL_DIR/$DAEMON_BINARY" ] && \
                [ ! -L "$INSTALL_DIR/$DAEMON_BINARY" ]
            [ "$(readlink "$INSTALL_DIR/$LEGACY_BINARY")" = \
                "$LEGACY_CURRENT_BASENAME/$LEGACY_BINARY" ]
            [ "$(readlink "$INSTALL_DIR/$LEGACY_DAEMON_BINARY")" = \
                "$INSTALL_DIR/$LEGACY_CURRENT_BASENAME/$LEGACY_DAEMON_BINARY" ]
            for canonical_fixture_unit in $SYSTEMD_UNITS; do
                [ ! -e "$SYSTEMD_USER_DIR/$canonical_fixture_unit" ] && \
                    [ ! -L "$SYSTEMD_USER_DIR/$canonical_fixture_unit" ]
            done
            [ ! -e "$INSTALL_DIR/.mitsuro-systemd-managed" ] && \
                [ ! -L "$INSTALL_DIR/.mitsuro-systemd-managed" ]
            self_unit_active "$LEGACY_SERVE_UNIT"
            self_unit_enabled "$LEGACY_SERVE_UNIT"
        else
            [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = \
                ".mitsuro-releases/$candidate_release_id" ]
            [ "$("$INSTALL_DIR/$BINARY")" = previous-success-candidate ]
            [ "$("$INSTALL_DIR/$COMPAT_BINARY")" = previous-success-candidate ]
            [ "$("$INSTALL_DIR/$DAEMON_BINARY")" = hive-previous-success-candidate ]
            [ "$("$INSTALL_DIR/$COMPAT_DAEMON_BINARY")" = \
                hive-previous-success-candidate ]
            self_unit_active mitsuro-serve.service
            self_unit_enabled mitsuro-serve.service
            if self_unit_active "$LEGACY_SERVE_UNIT" || \
                self_unit_enabled "$LEGACY_SERVE_UNIT"; then
                fail "Self-test retained previous service authority after migration."
                exit 1
            fi
        fi
        assert_no_activation_residue
    )

    run_previous_identity_release_fixture failure
    run_previous_identity_release_fixture success

    stage_self_test_release v1 one complete \
        1111111111111111111111111111111111111111111111111111111111111111
    v1_release_id=$RELEASE_ID
    v1_release_dir=$RELEASE_DIR
    stage_unix_release "$self_root/payload-one"
    [ -z "$RELEASE_STAGE" ]
    chmod 0755 "$v1_release_dir/mitsuro"
    if stage_unix_release "$self_root/payload-one"; then
        fail "Self-test accepted a release with a changed primary mode."
        exit 1
    fi
    chmod 0555 "$v1_release_dir/mitsuro"
    [ -z "$RELEASE_STAGE" ]

    multiline_target=$(printf 'line-one\nline-two')
    trailing_newline_target=$(printf 'line-one\nline-two\nsentinel')
    trailing_newline_target=${trailing_newline_target%sentinel}
    assert_unmanaged_symlink_rollback "$multiline_target" multiline
    assert_unmanaged_symlink_rollback "$trailing_newline_target" trailing-newline

    marker_target="$self_root/marker-target"
    printf '%s\n' marker > "$marker_target"
    ln -s "$marker_target" "$INSTALL_DIR/.mitsuro-systemd-managed"
    if activate_unix_release; then fail "Self-test accepted a symlink marker."; exit 1; fi
    [ -L "$INSTALL_DIR/.mitsuro-systemd-managed" ]
    rm -f "$INSTALL_DIR/.mitsuro-systemd-managed"
    mkdir "$INSTALL_DIR/.mitsuro-systemd-managed"
    if activate_unix_release; then fail "Self-test accepted a directory marker."; exit 1; fi
    rmdir "$INSTALL_DIR/.mitsuro-systemd-managed"
    assert_direct_baseline

    for failure_point in \
        after-mitsuro-link \
        after-hive-link \
        after-compat-cli-link \
        after-atlas-link \
        after-compat-hive-link \
        after-mitsuro-hive.socket-link \
        after-mitsuro-hive.service-link \
        after-mitsuro-serve.service-link \
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
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    [ "$("$INSTALL_DIR/mitsuro")" = one ]
    [ "$("$INSTALL_DIR/$COMPAT_BINARY")" = one ]
    [ "$(readlink "$INSTALL_DIR/agent-browser")" = ".mitsuro-current/agent-browser" ]
    [ "$("$INSTALL_DIR/agent-browser")" = atlas-one ]
    [ "$("$INSTALL_DIR/$COMPAT_DAEMON_BINARY")" = hive-one ]
    regular_file_with_mode "$INSTALL_DIR/.mitsuro-systemd-managed" 600
    v1_marker_contents=$(sed -n '1p' "$INSTALL_DIR/.mitsuro-systemd-managed")
    [ "$SELF_HEALTH_CHECKS" -ge 2 ]
    [ "$SELF_HIVE_PING_CHECKS" -ge 2 ]
    [ "$SELF_RUNTIME_IDENTITY_CHECKS" -ge 4 ]
    [ "$SELF_SERVER_HEALTH_CHECKS" -ge 2 ]
    assert_no_activation_residue
    serve_only_legacy=false
    for legacy_candidate in "$RELEASES_DIR"/legacy-*; do
        if [ -f "$legacy_candidate/systemd/mitsuro-serve.service" ] && \
            [ ! -e "$legacy_candidate/systemd/mitsuro-hive.socket" ] && \
            [ ! -e "$legacy_candidate/systemd/mitsuro-hive.service" ]; then
            serve_only_legacy=true
        fi
    done
    [ "$serve_only_legacy" = true ]

    # Canonical installs created before the ownership marker shipped left the
    # exact Mitsuro unit set as regular files beside an already-managed release
    # pointer. Adopt only that complete, known set. The historical serve unit
    # may use an expanded home path and predate the Atlas sidecar environment.
    rm -f "$INSTALL_DIR/.mitsuro-systemd-managed"
    for adopt_fixture_unit in $SYSTEMD_UNITS; do
        rm -f "$SYSTEMD_USER_DIR/$adopt_fixture_unit"
        if [ "$adopt_fixture_unit" = mitsuro-serve.service ]; then
            sed \
                -e "s|^ExecStart=%h/.local/bin/.mitsuro-current/mitsuro|ExecStart=$INSTALL_DIR/.mitsuro-current/mitsuro|" \
                -e '/^Environment=MITSURO_AGENT_BROWSER_PATH=%h\/\.local\/bin\/\.mitsuro-current\/agent-browser$/d' \
                "$v1_release_dir/systemd/$adopt_fixture_unit" > \
                "$SYSTEMD_USER_DIR/$adopt_fixture_unit"
        else
            cp "$v1_release_dir/systemd/$adopt_fixture_unit" \
                "$SYSTEMD_USER_DIR/$adopt_fixture_unit"
        fi
        chmod 0644 "$SYSTEMD_USER_DIR/$adopt_fixture_unit"
        cp "$SYSTEMD_USER_DIR/$adopt_fixture_unit" \
            "$self_root/adopt-$adopt_fixture_unit.expected"
    done
    SELF_TEST_FAIL_POINT=after-mitsuro-hive.socket-link
    reset_self_systemd
    if activate_unix_release; then
        fail "Self-test expected canonical unit adoption rollback."
        exit 1
    fi
    SELF_TEST_FAIL_POINT=""
    [ -z "$SELF_LOG" ]
    [ ! -e "$INSTALL_DIR/.mitsuro-systemd-managed" ] && \
        [ ! -L "$INSTALL_DIR/.mitsuro-systemd-managed" ]
    for rolled_back_adopt_unit in $SYSTEMD_UNITS; do
        [ -f "$SYSTEMD_USER_DIR/$rolled_back_adopt_unit" ] && \
            [ ! -L "$SYSTEMD_USER_DIR/$rolled_back_adopt_unit" ]
        cmp -s "$self_root/adopt-$rolled_back_adopt_unit.expected" \
            "$SYSTEMD_USER_DIR/$rolled_back_adopt_unit"
    done
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue

    reset_self_systemd
    activate_unix_release
    for adopted_fixture_unit in $SYSTEMD_UNITS; do
        [ "$(readlink "$SYSTEMD_USER_DIR/$adopted_fixture_unit")" = \
            "$INSTALL_DIR/.mitsuro-current/systemd/$adopted_fixture_unit" ]
        rm -f "$self_root/adopt-$adopted_fixture_unit.expected"
    done
    regular_file_with_mode "$INSTALL_DIR/.mitsuro-systemd-managed" 600
    assert_no_activation_residue

    # A locally changed unit is not installer-owned. Refuse it before any
    # systemd transition and restore the complete byte-for-byte snapshot.
    rm -f "$INSTALL_DIR/.mitsuro-systemd-managed"
    for rejected_fixture_unit in $SYSTEMD_UNITS; do
        rm -f "$SYSTEMD_USER_DIR/$rejected_fixture_unit"
        cp "$v1_release_dir/systemd/$rejected_fixture_unit" \
            "$SYSTEMD_USER_DIR/$rejected_fixture_unit"
        chmod 0644 "$SYSTEMD_USER_DIR/$rejected_fixture_unit"
    done
    printf '%s\n' '# local customization' >> \
        "$SYSTEMD_USER_DIR/mitsuro-hive.service"
    cp "$SYSTEMD_USER_DIR/mitsuro-hive.service" \
        "$self_root/rejected-canonical-unit.expected"
    reset_self_systemd
    if activate_unix_release; then
        fail "Self-test accepted a customized unmarked canonical unit."
        exit 1
    fi
    [ -z "$SELF_LOG" ]
    [ ! -e "$INSTALL_DIR/.mitsuro-systemd-managed" ] && \
        [ ! -L "$INSTALL_DIR/.mitsuro-systemd-managed" ]
    for rejected_fixture_unit in $SYSTEMD_UNITS; do
        [ -f "$SYSTEMD_USER_DIR/$rejected_fixture_unit" ] && \
            [ ! -L "$SYSTEMD_USER_DIR/$rejected_fixture_unit" ]
    done
    cmp -s "$self_root/rejected-canonical-unit.expected" \
        "$SYSTEMD_USER_DIR/mitsuro-hive.service"
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue
    rm -f "$self_root/rejected-canonical-unit.expected"
    for restored_fixture_unit in $SYSTEMD_UNITS; do
        cp "$v1_release_dir/systemd/$restored_fixture_unit" \
            "$SYSTEMD_USER_DIR/$restored_fixture_unit"
        chmod 0644 "$SYSTEMD_USER_DIR/$restored_fixture_unit"
    done
    reset_self_systemd
    activate_unix_release
    assert_no_activation_residue

    # Older supervised installs linked units directly into the selected
    # immutable release. A valid ownership marker plus an exact match to the
    # current release is sufficient to migrate those links to .mitsuro-current.
    for legacy_managed_unit in $SYSTEMD_UNITS; do
        rm -f "$SYSTEMD_USER_DIR/$legacy_managed_unit"
        ln -s "$v1_release_dir/systemd/$legacy_managed_unit" \
            "$SYSTEMD_USER_DIR/$legacy_managed_unit"
    done
    reset_self_systemd
    activate_unix_release
    for migrated_unit in $SYSTEMD_UNITS; do
        [ "$(readlink "$SYSTEMD_USER_DIR/$migrated_unit")" = \
            "$INSTALL_DIR/.mitsuro-current/systemd/$migrated_unit" ]
    done
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    [ "$SELF_HEALTH_CHECKS" -ge 2 ]
    assert_no_activation_residue

    # A service can fail its first health sample while systemd applies its
    # configured restart policy. Activation should accept eventual stability.
    reset_self_systemd
    ACTIVE_UNITS=" mitsuro-serve.service"
    SELF_AFTER_RESTART=true
    SELF_FAIL_HEALTH_COUNT=1
    verify_previously_active
    [ "$SELF_HEALTH_CHECKS" -ge 3 ]

    # The marker does not authorize unrelated absolute unit links.
    rejected_unit="$SYSTEMD_USER_DIR/mitsuro-hive.socket"
    rejected_target="$self_root/unmanaged-mitsuro-hive.socket"
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
    ln -s "$INSTALL_DIR/.mitsuro-current/systemd/mitsuro-hive.socket" "$rejected_unit"
    assert_no_activation_residue

    stage_self_test_release v0.7.3-downgrade downgrade mitsuro-only \
        2222222222222222222222222222222222222222222222222222222222222222
    if activate_unix_release; then fail "Self-test accepted a supervised downgrade."; exit 1; fi
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue

    stage_self_test_release v2 two complete \
        3333333333333333333333333333333333333333333333333333333333333333
    v2_release_dir=$RELEASE_DIR
    reset_self_systemd
    SELF_FAIL_RELOAD_ONCE=true
    if activate_unix_release; then fail "Self-test expected daemon-reload rollback."; exit 1; fi
    case "$SELF_LOG" in
        '|daemon-reload|daemon-reload|restart:mitsuro-serve.service') ;;
        *) fail "Daemon-reload rollback ordering was incorrect: $SELF_LOG"; exit 1 ;;
    esac
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]

    reset_self_systemd
    SELF_ADD_DEPENDENCY=true
    SELF_FAIL_RESTART_ONCE=true
    if activate_unix_release; then fail "Self-test expected restart rollback."; exit 1; fi
    case "$SELF_LOG" in
        *'|restart:mitsuro-serve.service|stop:mitsuro-hive.socket|daemon-reload|restart:mitsuro-serve.service'*) ;;
        *) fail "Candidate dependency was not stopped before rollback reload: $SELF_LOG"; exit 1 ;;
    esac
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    regular_file_with_mode "$INSTALL_DIR/.mitsuro-systemd-managed" 600
    [ "$(sed -n '1p' "$INSTALL_DIR/.mitsuro-systemd-managed")" = "$v1_marker_contents" ]
    [ -d "$v2_release_dir" ]
    assert_no_activation_residue

    reset_self_systemd
    SELF_ADD_DEPENDENCY=true
    SELF_FAIL_HEALTH_COUNT=$ACTIVATION_HEALTH_ATTEMPTS
    if activate_unix_release; then fail "Self-test expected unhealthy-service rollback."; exit 1; fi
    case "$SELF_LOG" in
        *'|restart:mitsuro-serve.service|stop:mitsuro-hive.socket|daemon-reload|restart:mitsuro-serve.service'*) ;;
        *) fail "Health rollback ordering was incorrect: $SELF_LOG"; exit 1 ;;
    esac
    [ "$SELF_HEALTH_CHECKS" -ge 3 ]
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    regular_file_with_mode "$INSTALL_DIR/.mitsuro-systemd-managed" 600
    [ "$(sed -n '1p' "$INSTALL_DIR/.mitsuro-systemd-managed")" = "$v1_marker_contents" ]
    assert_no_activation_residue

    reset_self_systemd
    SELF_FAIL_RUNTIME_IDENTITY_COUNT=$ACTIVATION_HEALTH_ATTEMPTS
    if activate_unix_release; then
        fail "Self-test accepted a service running the wrong release executable."
        exit 1
    fi
    [ "$SELF_RUNTIME_IDENTITY_CHECKS" -ge "$ACTIVATION_HEALTH_ATTEMPTS" ]
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue

    reset_self_systemd
    SELF_FAIL_HIVE_PING_COUNT=$ACTIVATION_HEALTH_ATTEMPTS
    if activate_unix_release; then
        fail "Self-test accepted an unresponsive Hive runtime."
        exit 1
    fi
    [ "$SELF_HIVE_PING_CHECKS" -ge "$ACTIVATION_HEALTH_ATTEMPTS" ]
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue

    reset_self_systemd
    SELF_FAIL_SERVER_HEALTH_COUNT=$ACTIVATION_HEALTH_ATTEMPTS
    if activate_unix_release; then
        fail "Self-test accepted an unhealthy Mitsuro HTTP server."
        exit 1
    fi
    [ "$SELF_SERVER_HEALTH_CHECKS" -ge "$ACTIVATION_HEALTH_ATTEMPTS" ]
    [ "$(readlink "$INSTALL_DIR/.mitsuro-current")" = ".mitsuro-releases/$v1_release_id" ]
    assert_no_activation_residue

    printf './mitsuro\nsystemd/mitsuro-serve.service\n' | safe_member_paths
    if printf '../escape\n' | safe_member_paths; then fail "Self-test accepted traversal."; exit 1; fi
    printf '%s\n' '-rwxr-xr-x fixture' 'drwxr-xr-x systemd' | safe_tar_member_types
    if printf '%s\n' 'lrwxr-xr-x escape' | safe_tar_member_types; then fail "Self-test accepted tar symlink."; exit 1; fi
    printf '%s\n' 'Archive: fixture' '-rwxr-xr-x fixture' | safe_zip_member_types 1
    if printf '%s\n' '?rwxr-xr-x fixture' | safe_zip_member_types 1; then fail "Self-test accepted unknown ZIP type."; exit 1; fi
    EXT=tar.gz
    tar czf "$self_root/good.tar.gz" -C "$self_root/payload-one" .
    preflight_archive "$self_root/good.tar.gz"
    ln -s mitsuro "$self_root/payload-one/escape"
    tar czf "$self_root/symlink.tar.gz" -C "$self_root/payload-one" .
    if preflight_archive "$self_root/symlink.tar.gz"; then fail "Self-test accepted tar symlink entry."; exit 1; fi
    if command -v zip >/dev/null 2>&1 && command -v unzip >/dev/null 2>&1; then
        mkdir "$self_root/zip-payload"
        printf '%s\n' zip > "$self_root/zip-payload/mitsuro.exe"
        (cd "$self_root/zip-payload" && zip -q "$self_root/good.zip" mitsuro.exe)
        EXT=zip
        preflight_archive "$self_root/good.zip"
        printf '%s\n' wrong-name > "$self_root/zip-payload/other.exe"
        (cd "$self_root/zip-payload" && zip -q "$self_root/wrong-name.zip" other.exe)
        if preflight_archive "$self_root/wrong-name.zip"; then
            fail "Self-test accepted a Windows ZIP without the exact canonical member."
            exit 1
        fi
        (cd "$self_root/zip-payload" && zip -q "$self_root/multiple.zip" mitsuro.exe other.exe)
        if preflight_archive "$self_root/multiple.zip"; then
            fail "Self-test accepted a Windows ZIP with multiple members."
            exit 1
        fi
        ln -s mitsuro.exe "$self_root/zip-payload/escape"
        (cd "$self_root/zip-payload" && zip -qy "$self_root/symlink.zip" escape)
        if preflight_archive "$self_root/symlink.zip"; then fail "Self-test accepted ZIP symlink entry."; exit 1; fi
    fi

    windows_payload="$self_root/windows-direct-payload"
    windows_install_dir="$self_root/windows-bin"
    mkdir -p "$windows_payload" "$windows_install_dir"
    printf '%s\n' windows-new > "$windows_payload/mitsuro.exe"
    cp "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    cp "$windows_payload/mitsuro.exe" "$self_root/windows-compat-payload.expected"
    INSTALL_DIR="$windows_install_dir"

    ln -s "$self_root/windows-payload.expected" "$windows_payload/$COMPAT_BINARY.exe"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted an unsafe Windows compatibility payload."
        exit 1
    fi
    [ "$LOCK_HELD" = false ]
    rm -f "$windows_payload/$COMPAT_BINARY.exe"

    printf '%s\n' mismatched-compatibility > "$windows_payload/$COMPAT_BINARY.exe"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted a non-identical Windows compatibility payload."
        exit 1
    fi
    [ "$LOCK_HELD" = false ]
    rm -f "$windows_payload/$COMPAT_BINARY.exe"

    mkdir "$INSTALL_DIR/mitsuro.exe"
    printf '%s\n' directory-sentinel > "$INSTALL_DIR/mitsuro.exe/sentinel"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted a directory Windows destination."
        exit 1
    fi
    [ "$LOCK_HELD" = true ]
    grep -Fqx directory-sentinel "$INSTALL_DIR/mitsuro.exe/sentinel"
    [ ! -e "$INSTALL_DIR/mitsuro.exe/mitsuro.exe" ] && [ ! -L "$INSTALL_DIR/mitsuro.exe/mitsuro.exe" ]
    cmp -s "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock
    remove_writable_tree "$INSTALL_DIR/mitsuro.exe"

    mkdir "$self_root/windows-link-target"
    printf '%s\n' symlink-sentinel > "$self_root/windows-link-target/sentinel"
    ln -s "$self_root/windows-link-target" "$INSTALL_DIR/mitsuro.exe"
    if install_windows_direct "$windows_payload"; then
        fail "Self-test accepted a symlink Windows destination."
        exit 1
    fi
    [ "$LOCK_HELD" = true ]
    [ -L "$INSTALL_DIR/mitsuro.exe" ]
    [ "$(readlink "$INSTALL_DIR/mitsuro.exe")" = "$self_root/windows-link-target" ]
    grep -Fqx symlink-sentinel "$self_root/windows-link-target/sentinel"
    [ ! -e "$self_root/windows-link-target/mitsuro.exe" ] && [ ! -L "$self_root/windows-link-target/mitsuro.exe" ]
    cmp -s "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock
    rm -f "$INSTALL_DIR/mitsuro.exe"

    printf '%s\n' windows-old > "$INSTALL_DIR/mitsuro.exe"
    cp "$INSTALL_DIR/mitsuro.exe" "$self_root/windows-destination.expected"
    SELF_TEST_FAIL_POINT=windows-before-publish
    if install_windows_direct "$windows_payload"; then
        fail "Self-test expected Windows pre-publication rollback."
        exit 1
    fi
    SELF_TEST_FAIL_POINT=""
    [ "$LOCK_HELD" = true ]
    cmp -s "$INSTALL_DIR/mitsuro.exe" "$self_root/windows-destination.expected"
    cmp -s "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock

    printf '%s\n' windows-pair-old-canonical > "$INSTALL_DIR/mitsuro.exe"
    printf '%s\n' windows-pair-old-compatibility > \
        "$INSTALL_DIR/$COMPAT_BINARY.exe"
    cp "$INSTALL_DIR/mitsuro.exe" "$self_root/windows-pair-canonical.expected"
    cp "$INSTALL_DIR/$COMPAT_BINARY.exe" \
        "$self_root/windows-pair-compatibility.expected"
    SELF_TEST_FAIL_POINT=windows-after-canonical-publish
    if install_windows_direct "$windows_payload"; then
        fail "Self-test expected Windows command-pair publication rollback."
        exit 1
    fi
    SELF_TEST_FAIL_POINT=""
    [ "$LOCK_HELD" = true ]
    cmp -s "$INSTALL_DIR/mitsuro.exe" \
        "$self_root/windows-pair-canonical.expected"
    cmp -s "$INSTALL_DIR/$COMPAT_BINARY.exe" \
        "$self_root/windows-pair-compatibility.expected"
    cmp -s "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    assert_no_windows_stage
    release_self_install_lock

    SELF_TEST_FAIL_WINDOWS_RESTORE=true
    SELF_TEST_FAIL_POINT=windows-after-canonical-publish
    if install_windows_direct "$windows_payload"; then
        fail "Self-test expected an injected Windows recovery failure."
        exit 1
    fi
    SELF_TEST_FAIL_POINT=""
    [ "$SELF_TEST_FAIL_WINDOWS_RESTORE" = false ]
    [ "$LOCK_HELD" = true ]
    [ -n "$windows_preserved_backup" ]
    [ -d "$windows_preserved_backup" ] && [ ! -L "$windows_preserved_backup" ]
    cmp -s "$windows_preserved_backup/canonical.file" \
        "$self_root/windows-pair-canonical.expected"
    cmp -s "$windows_preserved_backup/compatibility.file" \
        "$self_root/windows-pair-compatibility.expected"
    cmp -s "$INSTALL_DIR/$COMPAT_BINARY.exe" \
        "$self_root/windows-pair-compatibility.expected"
    cmp -s "$INSTALL_DIR/mitsuro.exe" "$self_root/windows-payload.expected"
    cp "$windows_preserved_backup/canonical.file" "$INSTALL_DIR/mitsuro.exe"
    chmod 0755 "$INSTALL_DIR/mitsuro.exe"
    cmp -s "$INSTALL_DIR/mitsuro.exe" \
        "$self_root/windows-pair-canonical.expected"
    remove_writable_tree "$windows_preserved_backup"
    windows_preserved_backup=""
    WINDOWS_CANONICAL_DESTINATION=""
    WINDOWS_COMPAT_DESTINATION=""
    assert_no_windows_stage
    release_self_install_lock

    install_windows_direct "$windows_payload"
    [ "$LOCK_HELD" = true ]
    regular_file_with_mode "$INSTALL_DIR/mitsuro.exe" 755
    cmp -s "$INSTALL_DIR/mitsuro.exe" "$self_root/windows-payload.expected"
    cmp -s "$windows_payload/mitsuro.exe" "$self_root/windows-payload.expected"
    regular_file_with_mode "$INSTALL_DIR/$COMPAT_BINARY.exe" 755
    cmp -s "$INSTALL_DIR/$COMPAT_BINARY.exe" "$self_root/windows-compat-payload.expected"
    assert_no_windows_stage
    release_self_install_lock
    [ "$WINDOWS_STATE_MIGRATION_REQUIRED" = false ]
    mkdir -p "$HOME/$LEGACY_CONFIG_BASENAME"
    detect_windows_state_cutover_requirement
    [ "$WINDOWS_STATE_MIGRATION_REQUIRED" = true ]

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
    RELEASES_DIR="$INSTALL_DIR/.mitsuro-releases"
    mkdir -p "$RELEASES_DIR"
    reset_self_systemd
    stage_self_test_release v0.7.3 compat mitsuro-only \
        0000000000000000000000000000000000000000000000000000000000000000
    activate_unix_release
    [ "$("$INSTALL_DIR/mitsuro")" = compat ]
    [ "$("$INSTALL_DIR/$COMPAT_BINARY")" = compat ]
    [ ! -e "$INSTALL_DIR/mitsuro-hive" ] && [ ! -L "$INSTALL_DIR/mitsuro-hive" ]

    # Prove the previous service authority is stopped before the source is
    # read, source DB/WAL digests survive the Rust-owned migration unchanged,
    # rollback preserves the old root and quarantines the failed canonical
    # root, and a retry starts only canonical authority.
    HOME="$self_root/identity-migration-home"
    export HOME
    DEFAULT_INSTALL_DIR="$HOME/.local/bin"
    INSTALL_DIR="$DEFAULT_INSTALL_DIR"
    SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
    RELEASES_DIR="$INSTALL_DIR/.mitsuro-releases"
    legacy_fixture_root="$HOME/$LEGACY_CONFIG_BASENAME"
    mkdir -p "$RELEASES_DIR" "$SYSTEMD_USER_DIR" "$legacy_fixture_root"
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$legacy_fixture_root/$LEGACY_DATABASE_BASENAME" \
            "PRAGMA journal_mode=WAL; CREATE TABLE fixture (value TEXT NOT NULL); INSERT INTO fixture VALUES ('old-authority');" \
            >/dev/null
    else
        python3 - "$legacy_fixture_root/$LEGACY_DATABASE_BASENAME" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
connection.execute("PRAGMA journal_mode=WAL")
connection.execute("CREATE TABLE fixture (value TEXT NOT NULL)")
connection.execute("INSERT INTO fixture VALUES ('old-authority')")
connection.commit()
connection.close()
PY
    fi
    printf '#!/bin/sh\nprintf "%%s\\n" "identity-previous-cli"\n' > \
        "$INSTALL_DIR/$LEGACY_BINARY"
    printf '#!/bin/sh\nprintf "%%s\\n" "identity-previous-hive"\n' > \
        "$INSTALL_DIR/$LEGACY_DAEMON_BINARY"
    chmod 0755 \
        "$INSTALL_DIR/$LEGACY_BINARY" \
        "$INSTALL_DIR/$LEGACY_DAEMON_BINARY"
    printf '%s\n' '# previous service generation' > \
        "$SYSTEMD_USER_DIR/$LEGACY_SERVE_UNIT"

    # Use a deterministic procfs model so this test is independent of any
    # unrelated processes on the host running the installer self-test.
    PROC_ROOT="$self_root/proc-fixture"
    SELF_TEST_PROCFS=true
    self_uid=$(id -u)
    mkdir -p "$PROC_ROOT/7001/fd"
    printf 'Name:\tself-test-neutral\nState:\tS (sleeping)\nUid:\t%s\t%s\t%s\t%s\n' \
        "$self_uid" "$self_uid" "$self_uid" "$self_uid" > \
        "$PROC_ROOT/7001/status"
    ln -s /usr/bin/sh "$PROC_ROOT/7001/exe"
    LEGACY_STATE_ROOT=$legacy_fixture_root
    CANONICAL_STATE_ROOT="$HOME/$CANONICAL_CONFIG_BASENAME"
    STATE_MIGRATION_PENDING=true
    prove_legacy_state_quiescent "self-test neutral process"

    mkdir -p "$PROC_ROOT/7002/fd"
    printf 'Name:\tself-test-idle-old\nState:\tS (sleeping)\nUid:\t%s\t%s\t%s\t%s\n' \
        "$self_uid" "$self_uid" "$self_uid" "$self_uid" > \
        "$PROC_ROOT/7002/status"
    ln -s "$INSTALL_DIR/$LEGACY_RELEASES_BASENAME/fixture/$LEGACY_BINARY" \
        "$PROC_ROOT/7002/exe"
    if prove_legacy_state_quiescent "self-test idle previous executable"; then
        fail "Self-test accepted an idle previous-generation Mitsuro process."
        exit 1
    fi
    rm -rf "$PROC_ROOT/7002"

    mkdir -p "$PROC_ROOT/7004/fd"
    printf 'Name:\tself-test-idle-canonical\nState:\tS (sleeping)\nUid:\t%s\t%s\t%s\t%s\n' \
        "$self_uid" "$self_uid" "$self_uid" "$self_uid" > \
        "$PROC_ROOT/7004/status"
    ln -s "$INSTALL_DIR/.mitsuro-releases/fixture/mitsuro" \
        "$PROC_ROOT/7004/exe"
    if prove_legacy_state_quiescent "self-test idle canonical executable"; then
        fail "Self-test accepted an idle canonical Mitsuro process during cutover."
        exit 1
    fi
    rm -rf "$PROC_ROOT/7004"

    mkdir -p "$PROC_ROOT/7003/fd"
    printf 'Name:\tself-test-open-db\nState:\tS (sleeping)\nUid:\t%s\t%s\t%s\t%s\n' \
        "$self_uid" "$self_uid" "$self_uid" "$self_uid" > \
        "$PROC_ROOT/7003/status"
    ln -s /usr/bin/sleep "$PROC_ROOT/7003/exe"
    ln -s "$legacy_fixture_root/$LEGACY_DATABASE_BASENAME" \
        "$PROC_ROOT/7003/fd/9"
    if prove_legacy_state_quiescent "self-test open previous database"; then
        fail "Self-test accepted a same-user process holding previous SQLite state."
        exit 1
    fi
    rm -rf "$PROC_ROOT/7003"

    mkdir -p "$PROC_ROOT/7005/fd"
    printf 'Name:\tself-test-open-canonical-db\nState:\tS (sleeping)\nUid:\t%s\t%s\t%s\t%s\n' \
        "$self_uid" "$self_uid" "$self_uid" "$self_uid" > \
        "$PROC_ROOT/7005/status"
    ln -s /usr/bin/python3 "$PROC_ROOT/7005/exe"
    ln -s "$CANONICAL_STATE_ROOT/$CANONICAL_DATABASE_BASENAME" \
        "$PROC_ROOT/7005/fd/9"
    if prove_legacy_state_quiescent "self-test open canonical database"; then
        fail "Self-test accepted a same-user process holding canonical SQLite state."
        exit 1
    fi
    rm -rf "$PROC_ROOT/7005"

    stage_self_test_release v3 identity-migration complete \
        4444444444444444444444444444444444444444444444444444444444444444
    SELF_TEST_STATE_MIGRATION=true
    reset_self_systemd
    SELF_ACTIVE_UNITS=" $LEGACY_SERVE_UNIT"
    SELF_ENABLED_UNITS=" $LEGACY_SERVE_UNIT"
    SELF_FAIL_RELOAD_ONCE=true
    if activate_unix_release; then
        fail "Self-test expected post-migration reload rollback."
        exit 1
    fi
    expected_migration_rollback="|stop:$LEGACY_SERVE_UNIT|daemon-reload|daemon-reload|restart:$LEGACY_SERVE_UNIT"
    [ "$SELF_LOG" = "$expected_migration_rollback" ]
    [ "$SELF_HIVE_PING_CHECKS" -ge 2 ]
    [ "$SELF_RUNTIME_IDENTITY_CHECKS" -ge 4 ]
    [ "$SELF_SERVER_HEALTH_CHECKS" -ge 2 ]
    [ -d "$legacy_fixture_root" ]
    [ ! -e "$HOME/$CANONICAL_CONFIG_BASENAME" ]
    [ -f "$STATE_MIGRATION_RECORD/source-manifest" ]
    grep -Fqx "source=$legacy_fixture_root" "$STATE_MIGRATION_RECORD/source-manifest"
    grep -Fqx "database=$STATE_SOURCE_DB_DIGEST" "$STATE_MIGRATION_RECORD/source-manifest"
    grep -Fqx "wal=$STATE_SOURCE_WAL_DIGEST" "$STATE_MIGRATION_RECORD/source-manifest"
    verify_legacy_sqlite_unchanged
    sqlite_integrity_check "$legacy_fixture_root/$LEGACY_DATABASE_BASENAME" \
        "self-test rollback"
    find "$HOME" -maxdepth 1 -type d -name "$CANONICAL_CONFIG_BASENAME.failed.*" \
        -print -quit | grep -q .

    reset_self_systemd
    SELF_ACTIVE_UNITS=" $LEGACY_SERVE_UNIT"
    SELF_ENABLED_UNITS=" $LEGACY_SERVE_UNIT"
    activate_unix_release
    expected_migration_success="|stop:$LEGACY_SERVE_UNIT|daemon-reload|restart:mitsuro-serve.service|enable:mitsuro-serve.service|disable:$LEGACY_SERVE_UNIT"
    [ "$SELF_LOG" = "$expected_migration_success" ]
    [ -d "$legacy_fixture_root" ]
    [ -d "$HOME/$CANONICAL_CONFIG_BASENAME" ]
    identity_migration_receipt_valid
    self_identity_receipt="$CANONICAL_STATE_ROOT/.identity-migration-v2"
    self_valid_identity_receipt="$self_root/valid-identity-migration-v2"
    cp "$self_identity_receipt" "$self_valid_identity_receipt"
    assert_invalid_self_test_receipt() {
        self_invalid_receipt_label=$1
        if identity_migration_receipt_valid; then
            fail "Self-test accepted invalid v2 receipt: $self_invalid_receipt_label"
            exit 1
        fi
        cp "$self_valid_identity_receipt" "$self_identity_receipt"
        if ! identity_migration_receipt_valid; then
            fail "Self-test could not restore the valid v2 receipt after: $self_invalid_receipt_label"
            exit 1
        fi
    }

    printf 'version=1\nsource=%s\ncreated_unix=1\nrollback_preserved=true\n' \
        "$LEGACY_STATE_ROOT" > "$self_identity_receipt"
    assert_invalid_self_test_receipt "deprecated four-line receipt"

    sed 's/tree_sha256=0/tree_sha256=A/' \
        "$self_valid_identity_receipt" > "$self_identity_receipt"
    assert_invalid_self_test_receipt "uppercase tree digest"

    sed 's/wal_len=absent;wal_mtime_ns=absent/wal_len=1;wal_mtime_ns=absent/' \
        "$self_valid_identity_receipt" > "$self_identity_receipt"
    assert_invalid_self_test_receipt "half-present WAL stat tuple"

    sed 's/^created_unix=.*/created_unix=18446744073709551616/' \
        "$self_valid_identity_receipt" > "$self_identity_receipt"
    assert_invalid_self_test_receipt "created_unix beyond u64"

    sed \
        's/main_mtime_ns=1/main_mtime_ns=340282366920938463463374607431768211456/' \
        "$self_valid_identity_receipt" > "$self_identity_receipt"
    assert_invalid_self_test_receipt "SQLite mtime beyond u128"

    {
        sed -n '1p' "$self_valid_identity_receipt"
        sed -n '3p' "$self_valid_identity_receipt"
        sed -n '2p' "$self_valid_identity_receipt"
        sed -n '4,5p' "$self_valid_identity_receipt"
    } > "$self_identity_receipt"
    assert_invalid_self_test_receipt "reordered fields"

    {
        printf 'version=2\r\n'
        sed -n '2,5p' "$self_valid_identity_receipt"
    } > "$self_identity_receipt"
    assert_invalid_self_test_receipt "CRLF field"

    self_receipt_bytes=$(wc -c < "$self_valid_identity_receipt" | tr -d '[:space:]')
    self_receipt_without_lf=$((self_receipt_bytes - 1))
    dd if="$self_valid_identity_receipt" of="$self_identity_receipt" \
        bs=1 count="$self_receipt_without_lf" 2>/dev/null
    assert_invalid_self_test_receipt "missing final LF"

    cp "$self_valid_identity_receipt" "$self_identity_receipt"
    printf 'extra=forged\n' >> "$self_identity_receipt"
    assert_invalid_self_test_receipt "extra field"

    cp "$self_valid_identity_receipt" "$self_identity_receipt"
    dd if=/dev/zero bs=16384 count=1 >> "$self_identity_receipt" 2>/dev/null
    assert_invalid_self_test_receipt "receipt larger than 16 KiB"

    sqlite_integrity_check \
        "$HOME/$CANONICAL_CONFIG_BASENAME/$CANONICAL_DATABASE_BASENAME" \
        "self-test canonical"
    self_unit_active mitsuro-serve.service
    self_unit_enabled mitsuro-serve.service
    if self_unit_active "$LEGACY_SERVE_UNIT"; then
        fail "Self-test left previous and canonical service authorities active."
        exit 1
    fi
    if self_unit_enabled "$LEGACY_SERVE_UNIT"; then
        fail "Self-test left previous service enablement in place."
        exit 1
    fi
    SELF_TEST_STATE_MIGRATION=false
    reset_self_systemd
    SELF_ACTIVE_UNITS=" mitsuro-serve.service"
    activate_unix_release
    [ "$STATE_MIGRATION_PENDING" = false ]
    [ "$STATE_MIGRATION_RECEIPTED" = true ]
    [ "$SELF_LOG" = '|daemon-reload|restart:mitsuro-serve.service' ]
    echo "install.sh self-test passed"
)

install() {
    detect_platform

    VERSION="${VERSION:-$(get_latest_version)}"
    if [ -z "$VERSION" ]; then
        fail "Could not determine latest version."
        exit 1
    fi
    if ! valid_release_tag "$VERSION"; then
        fail "Release tag must look like v0.9.23, got: $VERSION"
        exit 2
    fi

    echo "Installing Mitsuro $VERSION for $PLATFORM..."
    ARCHIVE="mitsuro-$PLATFORM.$EXT"
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

    if [ "$EXT" = "zip" ] && [ "$WINDOWS_STATE_MIGRATION_REQUIRED" = true ]; then
        echo "Previous Mitsuro state was detected at $HOME/$LEGACY_CONFIG_BASENAME."
        echo "The Windows installer publishes binaries only; it does not migrate state."
        echo "Before normal startup, stop every Mitsuro CLI, TUI, desktop, server, and Hive process from both generations, then run:"
        echo "  mitsuro migrate-identity --confirm-offline"
        echo "Start Mitsuro normally only after that command succeeds and preserves the old root."
    else
        echo "Run 'mitsuro' to start."
    fi
    if [ "$EXT" != "zip" ] && \
        { [ "$STATE_MIGRATION_PERFORMED" = true ] || \
          [ "$STATE_MIGRATION_RECEIPTED" = true ]; }; then
        echo "Recovery warning: previous state and physical releases are rollback-only."
        echo "Never launch an archived binary or previous desktop app except through coordinated rollback."
        echo "First stop and prove quiescence of every canonical CLI, TUI, desktop, server, and Hive process."
    fi
    if [ "$INSTALLED_SYSTEMD_UNITS" = true ]; then
        echo "To supervise Hive and the self-hosted server:"
        echo "  systemctl --user enable --now mitsuro-hive.socket mitsuro-serve.service"
    fi
}

if [ "${1:-}" = "--self-test" ]; then
    run_self_test
else
    install
fi
