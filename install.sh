#!/bin/sh
set -e

# Krusty installer
# Usage: curl -fsSLO https://raw.githubusercontent.com/honeycomb-Technologies/Krusty/main/install.sh && sh install.sh

REPO="honeycomb-Technologies/Krusty"
BINARY="krusty"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS and architecture
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64) PLATFORM="x86_64-unknown-linux-gnu" ;;
                aarch64|arm64) PLATFORM="aarch64-unknown-linux-gnu" ;;
                *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            EXT="tar.gz"
            ;;
        Darwin)
            case "$ARCH" in
                x86_64) PLATFORM="x86_64-apple-darwin" ;;
                arm64) PLATFORM="aarch64-apple-darwin" ;;
                *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            EXT="tar.gz"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            PLATFORM="x86_64-pc-windows-msvc"
            EXT="zip"
            ;;
        *)
            echo "Unsupported OS: $OS"
            exit 1
            ;;
    esac
}

# Get latest release version
get_latest_version() {
    curl -sL "https://api.github.com/repos/$REPO/releases/latest" | \
        grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/'
}

# Download and install
install() {
    detect_platform

    VERSION="${VERSION:-$(get_latest_version)}"
    if [ -z "$VERSION" ]; then
        echo "Error: Could not determine latest version"
        exit 1
    fi

    echo "Installing Krusty $VERSION for $PLATFORM..."

    ARCHIVE="krusty-$PLATFORM.$EXT"
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$VERSION/$ARCHIVE"
    CHECKSUM_URL="$DOWNLOAD_URL.sha256"
    TMP_DIR="$(mktemp -d)"

    echo "Downloading from $DOWNLOAD_URL..."
    curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ARCHIVE"

    echo "Downloading checksum from $CHECKSUM_URL..."
    if ! curl -fsSL "$CHECKSUM_URL" -o "$TMP_DIR/$ARCHIVE.sha256"; then
        echo "Error: Checksum file is required but could not be downloaded."
        rm -rf "$TMP_DIR"
        exit 1
    fi

    echo "Verifying checksum..."
    cd "$TMP_DIR"
    if command -v sha256sum >/dev/null 2>&1; then
        if ! sha256sum -c "$ARCHIVE.sha256" >/dev/null 2>&1; then
            echo "Error: Checksum verification failed!"
            echo "The downloaded file may be corrupted. Please try again."
            rm -rf "$TMP_DIR"
            exit 1
        fi
    elif command -v shasum >/dev/null 2>&1; then
        # macOS uses shasum
        if ! shasum -a 256 -c "$ARCHIVE.sha256" >/dev/null 2>&1; then
            echo "Error: Checksum verification failed!"
            echo "The downloaded file may be corrupted. Please try again."
            rm -rf "$TMP_DIR"
            exit 1
        fi
    else
        echo "Error: sha256sum or shasum is required to verify downloads."
        rm -rf "$TMP_DIR"
        exit 1
    fi
    echo "Checksum verified."

    echo "Extracting..."
    if [ "$EXT" = "tar.gz" ]; then
        tar xzf "$ARCHIVE"
    else
        unzip -q "$ARCHIVE"
    fi

    echo "Installing to $INSTALL_DIR..."
    mkdir -p "$INSTALL_DIR"
    mv "$BINARY" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/$BINARY"

    rm -rf "$TMP_DIR"

    echo ""
    echo "Krusty installed successfully!"
    echo ""

    # Check if install dir is in PATH
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
}

install
