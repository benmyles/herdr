#!/usr/bin/env bash
# Build an optimized herdr-benmyles release and install it beside stock herdr.
# An existing herdr-benmyles on PATH is replaced; otherwise it goes next to the
# herdr found on PATH, or into ~/.local/bin. Stock herdr is never touched.
# Symlinks are followed so dotfile links remain intact while their current
# target is replaced atomically.
#
# The vendored libghostty-vt crate is built with Zig and requires Zig 0.16.0.
# Zig is resolved in this order:
#   1. the ZIG environment variable, when its version is 0.16.0
#   2. Homebrew's versioned zig@0.16 or current zig formula (macOS)
#   3. the system Zig, when its version is 0.16.0
#   4. a cached copy under ${XDG_CACHE_HOME:-~/.cache}/herdr/zig-* (macOS)
#   5. on macOS, downloading Zig 0.16.0 into the cache automatically
#
# Set HERDR_BENMYLES_INSTALL_TARGET to install to an explicit path, or
# HERDR_BENMYLES_BIN_DIR to install to HERDR_BENMYLES_BIN_DIR/herdr-benmyles.
#
# --upgrade then moves every running herdr-benmyles session onto the installed
# binary with `herdr-benmyles server upgrade` (live handoff): pane processes
# keep running and attached clients exit, so run herdr-benmyles again to
# reattach with the new client.
#
# --remote also cross-builds static Linux binaries (x86_64 and aarch64) of the
# same commit into ${XDG_CACHE_HOME:-~/.cache}/herdr-benmyles/remote, where
# `herdr-benmyles --remote` finds them to install on SSH hosts. It needs
# cargo-zigbuild and the *-unknown-linux-musl rustup targets. Override the
# platform list with HERDR_BENMYLES_REMOTE_TARGETS="linux-x86_64 linux-aarch64".
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

BUILD_REMOTE=0
UPGRADE_RUNNING=0
for arg in "$@"; do
    case "$arg" in
    --remote) BUILD_REMOTE=1 ;;
    --upgrade) UPGRADE_RUNNING=1 ;;
    *)
        echo "usage: ./install.sh [--remote] [--upgrade]" >&2
        exit 2
        ;;
    esac
done
ZIG_VERSION="0.16.0"
BIN_NAME="herdr-benmyles"
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
    if [[ -n "${HERDR_BENMYLES_INSTALL_TARGET:-}" ]]; then
        printf '%s\n' "$HERDR_BENMYLES_INSTALL_TARGET"
        return 0
    fi
    if [[ -n "${HERDR_BENMYLES_BIN_DIR:-}" ]]; then
        printf '%s/%s\n' "${HERDR_BENMYLES_BIN_DIR%/}" "$BIN_NAME"
        return 0
    fi

    local path_entry
    path_entry="$(type -P "$BIN_NAME" 2>/dev/null || true)"
    if [[ -n "$path_entry" ]]; then
        printf '%s\n' "$path_entry"
        return 0
    fi
    path_entry="$(type -P herdr 2>/dev/null || true)"
    if [[ -n "$path_entry" ]]; then
        printf '%s/%s\n' "$(dirname -- "$path_entry")" "$BIN_NAME"
        return 0
    fi
    printf '%s/.local/bin/%s\n' "$HOME" "$BIN_NAME"
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

# Stamp the commit so remote attach keeps SSH hosts on this exact build. A
# dirty tree gets a per-run suffix: its local and remote builds match each
# other but no other dirty build.
HERDR_BUILD_COMMIT="$(git -C "$ROOT_DIR" rev-parse --short=12 HEAD)"
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=no)" ]]; then
    HERDR_BUILD_COMMIT="$HERDR_BUILD_COMMIT-dirty-$(date +%s)"
fi
export HERDR_BUILD_COMMIT

remote_rust_target() {
    case "$1" in
    linux-x86_64) printf 'x86_64-unknown-linux-musl\n' ;;
    linux-aarch64) printf 'aarch64-unknown-linux-musl\n' ;;
    *)
        echo "error: unsupported remote platform: $1" >&2
        exit 1
        ;;
    esac
}

build_remote_binaries() {
    if ! command -v cargo-zigbuild >/dev/null 2>&1; then
        echo "error: --remote needs cargo-zigbuild: cargo install cargo-zigbuild --locked" >&2
        exit 1
    fi
    local cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/$BIN_NAME/remote"
    local platform rust_target cache_dir
    for platform in ${HERDR_BENMYLES_REMOTE_TARGETS:-linux-x86_64 linux-aarch64}; do
        rust_target="$(remote_rust_target "$platform")"
        if ! rustup target list --installed | grep -qx "$rust_target"; then
            rustup target add "$rust_target"
        fi
        echo "building $BIN_NAME for $platform ($rust_target)"
        cargo zigbuild --release --locked --target "$rust_target"
        cache_dir="$cache_root/$platform"
        mkdir -p "$cache_dir"
        install -m 0755 "$ROOT_DIR/target/$rust_target/release/$BIN_NAME" "$cache_dir/.$BIN_NAME.tmp"
        mv -f -- "$cache_dir/.$BIN_NAME.tmp" "$cache_dir/$BIN_NAME"
        printf '%s\n' "$HERDR_BUILD_COMMIT" >"$cache_dir/build"
        echo "cached $platform build at $cache_dir/$BIN_NAME"
    done
}

echo "building $BIN_NAME release $HERDR_BUILD_COMMIT with $("$ZIG" version)"
cargo build --release --locked
if [[ "$BUILD_REMOTE" == 1 ]]; then
    build_remote_binaries
fi

BUILD_BINARY="$ROOT_DIR/target/release/$BIN_NAME"
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

TEMP_BINARY="$(mktemp "$INSTALL_DIR/.$BIN_NAME-install.XXXXXX")"
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
if [[ "$UPGRADE_RUNNING" == 1 ]]; then
    "$INSTALL_TARGET" server upgrade
else
    echo "running $BIN_NAME servers keep their current binary; move them onto this build with \`$BIN_NAME server upgrade\`"
fi
