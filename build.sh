#!/usr/bin/env bash
# Release builds, copied to build/.
#   ./build.sh            native build
#   ./build.sh arm64      cross build for 64 bit ARM Linux (Raspberry Pi 5)
#   ./build.sh all        both
#   ./build.sh clean      remove target/ and everything in build/
set -euo pipefail

cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"

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

clean() {
    cargo clean
    find build -mindepth 1 ! -name '.keep' -delete 2>/dev/null || true
    echo "cleaned target/ and build/"
}

mkdir -p build
case "${1:-native}" in
    native) build_native ;;
    arm64)  build_arm64 ;;
    all)    build_native; build_arm64 ;;
    clean)  clean ;;
    *)      echo "Usage: $0 [native|arm64|all|clean]" >&2; exit 1 ;;
esac
