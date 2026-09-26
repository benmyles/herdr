#!/usr/bin/env bash
# Build an optimized herdr release and replace the herdr executable selected
# by PATH. Symlinks are followed so package-manager or dotfile links remain
# intact while their current target is replaced atomically.
#
# The vendored libghostty-vt crate is built with Zig and requires Zig 0.16.0.
# Zig is resolved in this order:
#   1. the ZIG environment variable, when its version is 0.16.0
#   2. Homebrew's versioned zig@0.16 or current zig formula (macOS)
#   3. the system Zig, when its version is 0.16.0
#   4. a cached copy under ${XDG_CACHE_HOME:-~/.cache}/herdr/zig-* (macOS)
#   5. on macOS, downloading Zig 0.16.0 into the cache automatically
#
# Set HERDR_INSTALL_TARGET to test or install to an explicit path. The older
# HERDR_BIN_DIR override remains supported and installs to HERDR_BIN_DIR/herdr.
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ZIG_VERSION="0.16.0"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/herdr"

find_zig() {
    local env_candidate="${ZIG:-}"
    if [[ -n "$env_candidate" ]] &&
        [[ "$("$env_candidate" version 2>/dev/null || true)" == "$ZIG_VERSION" ]]; then
        printf '%s\n' "$env_candidate"
        return 0
    fi

    local homebrew_candidate
    for homebrew_candidate in \
        "/opt/homebrew/opt/zig@0.16/bin/zig" \
        "/usr/local/opt/zig@0.16/bin/zig" \
        "/opt/homebrew/opt/zig/bin/zig" \
        "/usr/local/opt/zig/bin/zig"; do
        if [[ -x "$homebrew_candidate" ]] &&
            [[ "$("$homebrew_candidate" version 2>/dev/null || true)" == "$ZIG_VERSION" ]]; then
            printf '%s\n' "$homebrew_candidate"
            return 0
        fi
    done

    local system_candidate
    if system_candidate="$(command -v zig 2>/dev/null || true)" &&
        [[ -n "$system_candidate" ]] && [[ "$(zig version 2>/dev/null || true)" == "$ZIG_VERSION" ]]; then
        printf '%s\n' "$system_candidate"
        return 0
    fi

    if [[ "$(uname -s)" == "Darwin" ]]; then
        local arch
        case "$(uname -m)" in
            arm64) arch="aarch64" ;;
            x86_64) arch="x86_64" ;;
            *)
                echo "error: unsupported architecture for macOS Zig download: $(uname -m)" >&2
                exit 1
                ;;
        esac

        local zig_dir="$CACHE_DIR/zig-$arch-macos-$ZIG_VERSION"
        if [[ ! -x "$zig_dir/zig" ]] || [[ "$("$zig_dir/zig" version 2>/dev/null || true)" != "$ZIG_VERSION" ]]; then
            local tarball="$CACHE_DIR/zig-$arch-macos-$ZIG_VERSION.tar.xz"
            echo "downloading Zig $ZIG_VERSION to $zig_dir" >&2
            mkdir -p "$CACHE_DIR"
            if [[ ! -f "$tarball" ]]; then
                curl -fL "https://ziglang.org/download/$ZIG_VERSION/zig-$arch-macos-$ZIG_VERSION.tar.xz" \
                    -o "$tarball.tmp"
                mv "$tarball.tmp" "$tarball"
            fi
            rm -rf "$zig_dir"
            mkdir -p "$zig_dir"
            tar -xJf "$tarball" --strip-components=1 -C "$zig_dir"
        fi

        # Zig links against the Command Line Tools SDK. When a full
        # Xcode SDK is selected, the parent shell exports DEVELOPER_DIR
        # before the build so linking works.

        printf '%s\n' "$zig_dir/zig"
        return 0
    fi

    echo "error: Zig $ZIG_VERSION is required to build the vendored libghostty-vt," >&2
    echo "       but no compatible Zig was found. Install it from" >&2
    echo "       https://ziglang.org/download/ or set ZIG=/path/to/zig." >&2
    exit 1
}

find_install_path() {
    if [[ -n "${HERDR_INSTALL_TARGET:-}" ]]; then
        printf '%s\n' "$HERDR_INSTALL_TARGET"
        return 0
    fi
    if [[ -n "${HERDR_BIN_DIR:-}" ]]; then
        printf '%s/herdr\n' "${HERDR_BIN_DIR%/}"
        return 0
    fi

    local path_entry
    path_entry="$(type -P herdr 2>/dev/null || true)"
    if [[ -z "$path_entry" ]]; then
        echo "error: herdr is not installed in PATH." >&2
        echo "       Set HERDR_INSTALL_TARGET=/absolute/path/to/herdr and retry." >&2
        exit 1
    fi
    printf '%s\n' "$path_entry"
}

resolve_install_target() {
    local target="$1"
    local target_dir
    local link
    local depth=0

    target_dir="$(dirname -- "$target")"
    mkdir -p "$target_dir"
    target_dir="$(cd -P -- "$target_dir" && pwd)"
    target="$target_dir/$(basename -- "$target")"

    while [[ -L "$target" ]]; do
        depth=$((depth + 1))
        if ((depth > 40)); then
            echo "error: too many symlinks while resolving $1" >&2
            exit 1
        fi
        link="$(readlink "$target")"
        if [[ "$link" == /* ]]; then
            target="$link"
        else
            target="$(dirname -- "$target")/$link"
        fi
        target_dir="$(cd -P -- "$(dirname -- "$target")" && pwd)"
        target="$target_dir/$(basename -- "$target")"
    done

    printf '%s\n' "$target"
}

cd "$ROOT_DIR"
ZIG="$(find_zig)"
export ZIG
PATH_ENTRY="$(find_install_path)"
INSTALL_TARGET="$(resolve_install_target "$PATH_ENTRY")"

# Zig links against the Command Line Tools SDK on macOS. When a full
# Xcode SDK is selected by xcode-select, set DEVELOPER_DIR so both the zig
# build and the final link use Command Line Tools.
if [[ "$(uname -s)" == "Darwin" ]] && [[ -d "/Library/Developer/CommandLineTools/SDKs" ]]; then
    export DEVELOPER_DIR="/Library/Developer/CommandLineTools"
fi

echo "building herdr release with $("$ZIG" version)"
cargo build --release --locked

BUILD_BINARY="$ROOT_DIR/target/release/herdr"
INSTALL_DIR="$(dirname -- "$INSTALL_TARGET")"
if [[ ! -w "$INSTALL_DIR" ]]; then
    echo "error: install directory is not writable: $INSTALL_DIR" >&2
    exit 1
fi

TEMP_BINARY=""
cleanup() {
    if [[ -n "$TEMP_BINARY" ]]; then
        rm -f -- "$TEMP_BINARY"
    fi
}
trap cleanup EXIT

TEMP_BINARY="$(mktemp "$INSTALL_DIR/.herdr-install.XXXXXX")"
install -m 0755 "$BUILD_BINARY" "$TEMP_BINARY"
"$TEMP_BINARY" --version >/dev/null

echo "installing to $PATH_ENTRY"
if [[ "$PATH_ENTRY" != "$INSTALL_TARGET" ]]; then
    echo "resolved install target: $INSTALL_TARGET"
fi
mv -f -- "$TEMP_BINARY" "$INSTALL_TARGET"
TEMP_BINARY=""
trap - EXIT

echo "installed $("$INSTALL_TARGET" --version)"
echo "running Herdr servers keep their current binary until they are stopped and restarted"
