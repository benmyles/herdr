#!/usr/bin/env bash
# Build an optimized herdr release and install it to ~/bin/herdr.
#
# The vendored libghostty-vt crate is built with Zig and requires Zig 0.15.2.
# Zig is resolved in this order:
#   1. the ZIG environment variable, when its version is 0.15.2
#   2. a cached copy under ${XDG_CACHE_HOME:-~/.cache}/herdr/zig-* (macOS)
#   3. the system Zig, when its version is 0.15.2
#   4. on macOS, downloading Zig 0.15.2 into the cache automatically
#
# Install destination can be overridden with HERDR_BIN_DIR (default ~/bin).
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
BIN_DIR=${HERDR_BIN_DIR:-"$HOME/bin"}
ZIG_VERSION="0.15.2"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/herdr"

find_zig() {
    local env_candidate="${ZIG:-}"
    if [[ -n "$env_candidate" ]] &&
        [[ "$("$env_candidate" version 2>/dev/null || true)" == "$ZIG_VERSION" ]]; then
        printf '%s\n' "$env_candidate"
        return 0
    fi

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
            echo "downloading Zig $ZIG_VERSION to $zig_dir"
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

        # Zig 0.15.2 links against the Command Line Tools SDK. When a full
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

cd "$ROOT_DIR"
export ZIG="$(find_zig)"

# Zig 0.15.2 links against the Command Line Tools SDK on macOS. When a full
# Xcode SDK is selected by xcode-select, set DEVELOPER_DIR so both the zig
# build and the final link use Command Line Tools.
if [[ "$(uname -s)" == "Darwin" ]] && [[ -d "/Library/Developer/CommandLineTools/SDKs" ]]; then
    export DEVELOPER_DIR="/Library/Developer/CommandLineTools"
fi

echo "building herdr release with $("$ZIG" version)"
cargo build --release --locked

echo "installing to $BIN_DIR/herdr"
mkdir -p "$BIN_DIR"
cp target/release/herdr "$BIN_DIR/.herdr-next"
mv -f "$BIN_DIR/.herdr-next" "$BIN_DIR/herdr"
