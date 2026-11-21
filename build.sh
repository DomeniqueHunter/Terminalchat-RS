#!/usr/bin/env bash
# Release builds, copied to build/.
#   ./build.sh            native build
#   ./build.sh arm64      cross build for 64 bit ARM Linux (Raspberry Pi 5)
#   ./build.sh windows    cross build for 64 bit Windows (.exe, needs mingw-w64)
#   ./build.sh all        native and arm64
#   ./build.sh clean      remove target/ and everything in build/
#   --restart             after a native build: restart the client with the new binary
#                         (inside its tmux session if one runs, otherwise in this terminal)
set -euo pipefail

cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"

# rustup installs cargo into ~/.cargo/bin, which is not in PATH of every shell
if ! command -v cargo >/dev/null 2>&1 && [[ -f "${HOME}/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    . "${HOME}/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Run './setup.sh' to install Rust." >&2
    exit 1
fi

ARM_TARGET="aarch64-unknown-linux-gnu"
ARM_LINKER="aarch64-linux-gnu-gcc"
BIN="terminalchat"

build_native() {
    cargo build --release
    cp --remove-destination "target/release/${BIN}" "build/${BIN}-$(uname -m)"
    chmod +x "build/${BIN}-$(uname -m)"
    echo "-> build/${BIN}-$(uname -m)"
}

build_arm64() {
    if ! command -v "${ARM_LINKER}" >/dev/null 2>&1; then
        echo "${ARM_LINKER} not found. Install it (Debian/Ubuntu: apt install gcc-aarch64-linux-gnu)" >&2
        echo "or run './build.sh' directly on the Raspberry Pi." >&2
        exit 1
    fi
    rustup target add "${ARM_TARGET}"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="${ARM_LINKER}"
    export CC_aarch64_unknown_linux_gnu="${ARM_LINKER}"
    cargo build --release --target "${ARM_TARGET}"
    cp --remove-destination "target/${ARM_TARGET}/release/${BIN}" "build/${BIN}-aarch64"
    chmod +x "build/${BIN}-aarch64"
    echo "-> build/${BIN}-aarch64"
}

WIN_TARGET="x86_64-pc-windows-gnu"
WIN_LINKER="x86_64-w64-mingw32-gcc"

build_windows() {
    if ! command -v "${WIN_LINKER}" >/dev/null 2>&1; then
        echo "${WIN_LINKER} not found. Install it (Debian/Ubuntu: apt install gcc-mingw-w64-x86-64)" >&2
        exit 1
    fi
    rustup target add "${WIN_TARGET}"
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${WIN_LINKER}"
    export CC_x86_64_pc_windows_gnu="${WIN_LINKER}"
    cargo build --release --target "${WIN_TARGET}"
    cp --remove-destination "target/${WIN_TARGET}/release/${BIN}.exe" "build/${BIN}-x86_64.exe"
    echo "-> build/${BIN}-x86_64.exe"
}

# Ends the running client cleanly (SIGTERM, it saves its files) and starts the new binary in the
# same tmux pane, so an attached window shows the new version at once.
restart_client() {
    local session="terminalchat-rs"
    tmux_cmd() {
        env -u TMUX tmux -L "${session}" -f "tmux.conf" "$@"
    }
    if ! command -v tmux >/dev/null 2>&1 || ! tmux_cmd has-session -t "${session}" 2>/dev/null; then
        echo "starting terminalchat"
        exec ./terminalchat
    fi
    local pid
    pid="$(tmux_cmd display-message -p -t "${session}" '#{pane_pid}')"
    tmux_cmd set-option -p -t "${session}" remain-on-exit on
    kill -TERM "${pid}" 2>/dev/null || true
    pkill -TERM -P "${pid}" 2>/dev/null || true
    for _ in $(seq 50); do
        kill -0 "${pid}" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "${pid}" 2>/dev/null; then
        echo "terminalchat did not stop within 5 s, killing it" >&2
        tmux_cmd respawn-pane -k -t "${session}"
    else
        tmux_cmd respawn-pane -t "${session}"
    fi
    tmux_cmd set-option -p -t "${session}" remain-on-exit off
    echo "terminalchat restarted in its tmux session"
}

clean() {
    cargo clean
    find build -mindepth 1 ! -name '.keep' -delete 2>/dev/null || true
    echo "cleaned target/ and build/"
}

usage() {
    echo "Usage: $0 [native|arm64|windows|all|clean] [--restart]" >&2
    exit 1
}

TASK="native"
RESTART=false
for arg in "$@"; do
    case "${arg}" in
        native|arm64|windows|all|clean) TASK="${arg}" ;;
        --restart) RESTART=true ;;
        *) usage ;;
    esac
done

mkdir -p build
case "${TASK}" in
    native) build_native ;;
    arm64)  build_arm64 ;;
    windows) build_windows ;;
    all)    build_native; build_arm64 ;;
    clean)  clean ;;
esac

if [[ "${RESTART}" == true ]]; then
    case "${TASK}" in
        native|all) restart_client ;;
        *) echo "--restart works only with a native build" >&2 ;;
    esac
fi
