#!/usr/bin/env bash
set -euo pipefail

BASE_DIR="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
MIN_RUST="1.86"
LINK_PATH="${HOME}/.local/bin/terminalchat"

SKIP_PACKAGES=0
SKIP_RUST=0
SKIP_BUILD=0
SKIP_LINK=0
ASSUME_YES=0

usage() {
    cat <<EOF
Usage: ./setup.sh [options]

Installs system packages (tmux, C compiler, curl), the Rust toolchain
(via rustup, if cargo is missing or too old) and builds the client with
./build.sh for this machine and links the launcher to ~/.local/bin/terminalchat
so that you can start it from anywhere with 'terminalchat'.

Options:
  --skip-packages   do not install system packages
  --skip-rust       do not install or update Rust
  --skip-build      do not build, only prepare
  --skip-link       do not link the launcher into ~/.local/bin
  -y, --yes         do not ask before installing anything
  -h, --help        show this help
EOF
}

info() {
    printf '\033[1m==> %s\033[0m\n' "$*"
}

fail() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --skip-packages) SKIP_PACKAGES=1 ;;
            --skip-rust) SKIP_RUST=1 ;;
            --skip-build) SKIP_BUILD=1 ;;
            --skip-link) SKIP_LINK=1 ;;
            -y|--yes) ASSUME_YES=1 ;;
            -h|--help) usage; exit 0 ;;
            *) usage >&2; fail "unknown option: $1" ;;
        esac
        shift
    done
}

as_root() {
    if [[ "$(id -u)" -eq 0 ]]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        fail "need root rights for: $* (sudo not found)"
    fi
}

ask() {
    [[ "${ASSUME_YES}" -eq 1 ]] && return 0
    local answer
    read -r -p "$1 [Y/n] " answer
    [[ "${answer,,}" != "n" ]]
}

detect_package_command() {
    if command -v pacman >/dev/null 2>&1; then
        echo "pacman -S --needed tmux base-devel curl"
    elif command -v apt-get >/dev/null 2>&1; then
        echo "apt-get install tmux build-essential curl"
    elif command -v dnf >/dev/null 2>&1; then
        echo "dnf install tmux gcc curl"
    elif command -v zypper >/dev/null 2>&1; then
        echo "zypper install tmux gcc curl"
    else
        echo ""
    fi
}

install_packages() {
    local package_command
    package_command="$(detect_package_command)"
    if [[ -z "${package_command}" ]]; then
        info "No supported package manager found - please install tmux, a C compiler and curl manually"
        return
    fi

    info "System packages: ${package_command}"
    if ! ask "Install now?"; then
        info "Skipping system packages"
        return
    fi

    local args
    read -r -a args <<< "${package_command}"
    case "${args[0]}" in
        apt-get)
            as_root apt-get update
            as_root "${args[@]}" -y
            ;;
        pacman)
            as_root "${args[@]}" --noconfirm
            ;;
        *)
            as_root "${args[@]}" -y
            ;;
    esac
}

load_cargo_env() {
    if [[ -f "${HOME}/.cargo/env" ]]; then
        # shellcheck disable=SC1091
        . "${HOME}/.cargo/env"
    fi
}

rust_version() {
    rustc --version 2>/dev/null | awk '{print $2}'
}

rust_is_recent() {
    local current
    current="$(rust_version)"
    [[ -n "${current}" ]] && [[ "$(printf '%s\n%s\n' "${MIN_RUST}" "${current}" | sort -V | head -n1)" == "${MIN_RUST}" ]]
}

install_rust() {
    load_cargo_env
    if command -v cargo >/dev/null 2>&1 && rust_is_recent; then
        info "Rust $(rust_version)"
        return
    fi

    if command -v rustup >/dev/null 2>&1; then
        info "Rust is older than ${MIN_RUST} - updating with rustup"
        ask "Run 'rustup update stable'?" || fail "Rust ${MIN_RUST} or newer is required"
        rustup update stable
    else
        info "Rust ${MIN_RUST}+ not found - installing with rustup (https://rustup.rs)"
        command -v curl >/dev/null 2>&1 || fail "curl not found"
        ask "Install Rust into ~/.cargo now?" || fail "Rust ${MIN_RUST} or newer is required"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    fi
    load_cargo_env
    rust_is_recent || fail "Rust ${MIN_RUST} or newer is still not available"
    info "Rust $(rust_version)"
}

build() {
    info "Building (this takes a few minutes the first time)"
    "${BASE_DIR}/build.sh"
}

link_launcher() {
    local launcher="${BASE_DIR}/terminalchat"
    local current

    if [[ -L "${LINK_PATH}" ]]; then
        current="$(readlink -f "${LINK_PATH}" || true)"
        if [[ "${current}" == "${launcher}" ]]; then
            info "Link ${LINK_PATH} is already set"
            return
        fi
        info "${LINK_PATH} currently points to ${current:-a missing file}"
        if ! ask "Point it to ${launcher}?"; then
            info "Keeping the existing link"
            return
        fi
        ln -sfn "${launcher}" "${LINK_PATH}"
    elif [[ -e "${LINK_PATH}" ]]; then
        info "${LINK_PATH} exists and is not a link - not touching it"
        return
    else
        mkdir -p "$(dirname "${LINK_PATH}")"
        ln -s "${launcher}" "${LINK_PATH}"
    fi
    info "Linked ${LINK_PATH} -> ${launcher}"

    case ":${PATH}:" in
        *":$(dirname "${LINK_PATH}"):"*) ;;
        *) info "$(dirname "${LINK_PATH}") is not in your PATH - add it to ~/.profile to use 'terminalchat' everywhere" ;;
    esac
}

check_result() {
    if command -v tmux >/dev/null 2>&1; then
        info "tmux: $(tmux -V)"
    else
        info "tmux not installed - the client works, but /minimize is unavailable"
    fi
    info "Done. Start the client with: terminalchat (or ./terminalchat)"
}

main() {
    parse_args "$@"
    chmod +x "${BASE_DIR}/build.sh" "${BASE_DIR}/terminalchat" "${BASE_DIR}/purge.sh" "${BASE_DIR}/setup.sh"
    if [[ "${SKIP_PACKAGES}" -eq 0 ]]; then
        install_packages
    fi
    if [[ "${SKIP_RUST}" -eq 0 ]]; then
        install_rust
    else
        load_cargo_env
    fi
    if [[ "${SKIP_BUILD}" -eq 0 ]]; then
        command -v cargo >/dev/null 2>&1 || fail "cargo not found (run without --skip-rust)"
        build
    fi
    if [[ "${SKIP_LINK}" -eq 0 ]]; then
        link_launcher
    fi
    check_result
}

main "$@"
